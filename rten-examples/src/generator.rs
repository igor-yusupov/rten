use std::error::Error;
use std::fmt;

use rten::{Dimension, Input, Model, NodeId, Operators, Output};
use rten_tensor::prelude::*;
use rten_tensor::{NdTensor, Tensor};
use rten_text::tokenizers::Tokenizer;

/// Errors that occur when creating or running a [`Generator`].
#[derive(Debug)]
pub enum GeneratorError {
    /// An expected model input was not found.
    InputNotFound(String),

    /// An expected model output was not found.
    OutputNotFound(String),

    /// An input or output did not have the expected shape.
    ShapeMismatch(String),

    /// An error occurred while generating the next token.
    GenerateError(Box<dyn Error>),
}

impl fmt::Display for GeneratorError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            GeneratorError::InputNotFound(name) => write!(f, "model input not found: {}", name),
            GeneratorError::OutputNotFound(name) => write!(f, "model output not found: {}", name),
            GeneratorError::ShapeMismatch(err) => write!(f, "shape mismatch: {}", err),
            GeneratorError::GenerateError(err) => write!(f, "generation error: {}", err),
        }
    }
}

impl Error for GeneratorError {}

/// Key-value cache for a single layer of a transformer model.
struct KvCache {
    /// Input ID for this cache entry.
    input_id: NodeId,

    /// Output ID for this cache entry.
    output_id: NodeId,

    /// The cached keys and values, with shape [batch, heads, seq_len, size].
    cache: NdTensor<f32, 4>,
}

/// Generates a token sequence using an auto-regressive language model.
///
/// This is an iterator that runs the model on each call to [`Iterator::next`]
/// and yields a result containing the next token ID or an error.
pub struct Generator<'a> {
    model: &'a Model,

    /// Additional model inputs (eg. encoder outputs) passed to each model
    /// step.
    constant_inputs: Vec<(NodeId, Input<'a>)>,

    /// Additional model inputs computed using constant propagation. This
    /// effectively caches parts of the graph that don't change in each
    /// generation step. This is `None` if the cache is out of date.
    constant_prop_inputs: Option<Vec<(NodeId, Output)>>,

    /// Input token IDs for the next run of the model.
    input_ids: Vec<u32>,

    // Input node IDs
    input_ids_input: NodeId,
    attention_mask_input: Option<NodeId>,
    position_ids_input: Option<NodeId>,

    // Output node IDs
    logits_output: NodeId,

    /// Length of the sequence generated so far.
    seq_len: u32,

    /// Key-value cache.
    kv_cache: Vec<KvCache>,
}

impl<'a> Generator<'a> {
    /// Create a generator that iteratively produces tokens using a model.
    ///
    /// The model is expected to have the following inputs:
    ///
    ///  - `input_ids` - (batch, sequence) tensor of token IDs
    ///  - `attention_mask` - (batch, sequence) tensor of booleans
    ///  - `position_ids` - (batch, sequence) tensor of position indices
    ///  - `past_key_values.N.key` - (batch, head, past_seq_len, size) key vector cache
    ///    where `N` is the layer index
    ///  - `past_key_values.N.value` - (batch, head, past_key_values, size) value vector cache,
    ///    where `N` is the layer index
    ///
    /// The model is expected to have the following outputs:
    ///
    ///  - `logits` - output (batch, sequence, vocab) tensor of next token probabilities
    ///  - `present.N.key` - (batch, head, past_seq_len + 1, size) updated key vector cache
    ///  - `present.N.value` - (batch, head, past_seq_len + 1, size) updated value vector cache
    pub fn from_model(model: &'a Model) -> Result<Generator<'a>, GeneratorError> {
        let input_ids_input = model
            .find_node("input_ids")
            .ok_or(GeneratorError::InputNotFound("input_ids".to_string()))?;
        let attention_mask_input = model.find_node("attention_mask");
        let position_ids_input = model.find_node("position_ids");

        let logits_output = model
            .find_node("logits")
            .ok_or(GeneratorError::OutputNotFound("logits".to_string()))?;

        // Find inputs and corresponding outputs for key-value cache.
        let batch_size = 1;
        let mut kv_cache = Vec::new();
        for &input_id in model.input_ids() {
            let input_info = model
                .node_info(input_id)
                .ok_or(GeneratorError::InputNotFound(format!(
                    "input ID {}",
                    input_id
                )))?;
            let Some(name) = input_info.name() else {
                continue;
            };

            if !name.starts_with("past_key_values.") {
                continue;
            }

            if !name.ends_with(".key") && !name.ends_with(".value") {
                continue;
            }

            let [n_heads, size] = match input_info.shape().as_deref() {
                Some(&[_, Dimension::Fixed(n_heads), _, Dimension::Fixed(size)]) => [n_heads, size],
                _ => {
                    return Err(GeneratorError::ShapeMismatch(format!("input \"{}\" has unexpected shape. expected (batch, heads, past_seq_len, size) where `heads` and `size` are fixed", name)));
                }
            };

            let cache_type = if name.ends_with(".key") {
                "key"
            } else {
                "value"
            };

            let layer_index_start = "past_key_values.".len();
            let layer_index_str: String = name[layer_index_start..]
                .chars()
                .take_while(|ch| ch.is_ascii_digit())
                .collect();
            let Ok(layer_index) = layer_index_str.parse::<u32>() else {
                continue;
            };

            let output_name = format!("present.{}.{}", layer_index, cache_type);
            let output_id = model
                .find_node(&output_name)
                .ok_or(GeneratorError::OutputNotFound(output_name))?;

            kv_cache.push(KvCache {
                input_id,
                output_id,
                cache: NdTensor::zeros([batch_size, n_heads, 0 /* seq len */, size]),
            });
        }

        Ok(Generator {
            model,
            constant_inputs: Vec::new(),
            constant_prop_inputs: None,
            input_ids: vec![],
            input_ids_input,
            attention_mask_input,
            position_ids_input,
            logits_output,
            kv_cache,
            seq_len: 0,
        })
    }

    /// Set the initial sequence of tokens (aka. the prompt) passed to the model
    /// when it is first run.
    pub fn with_prompt(mut self, prompt: &'a [u32]) -> Self {
        self.input_ids = prompt.to_vec();
        self
    }

    /// Add a constant input which is provided to the model at each iteration.
    ///
    /// A common use case is to pass the outputs of an encoder model to
    /// an auto-regressive decoder.
    pub fn with_constant_input(mut self, input_id: NodeId, value: Input<'a>) -> Self {
        self.constant_prop_inputs = None;
        self.constant_inputs.push((input_id, value));
        self
    }

    /// Run the model and generate the next token.
    fn generate_next_token(&mut self) -> Result<u32, GeneratorError> {
        fn wrap_error<E>(e: E) -> GeneratorError
        where
            E: Into<Box<dyn Error>>,
        {
            GeneratorError::GenerateError(e.into())
        }

        let batch_size = 1;
        let input_ids: NdTensor<i32, 2> = self
            .input_ids
            .iter()
            .map(|id| *id as i32)
            .collect::<Tensor<_>>()
            .into_shape([batch_size, self.input_ids.len()]);

        let attention_mask = NdTensor::full([batch_size, self.input_ids.len()], 1i32);
        let position_ids = NdTensor::from_fn([batch_size, input_ids.len()], |[_batch, pos]| {
            self.seq_len as i32 + pos as i32
        });

        let mut model_inputs: Vec<(NodeId, Input)> =
            vec![(self.input_ids_input, input_ids.view().into())];

        if let Some(attention_mask_input) = self.attention_mask_input {
            model_inputs.push((attention_mask_input, attention_mask.view().into()));
        }

        if let Some(position_ids_input) = self.position_ids_input {
            model_inputs.push((position_ids_input, position_ids.view().into()));
        }

        // Propagate constants on the first run.
        if self.constant_prop_inputs.is_none() {
            let inputs =
                match self
                    .model
                    .partial_run(&self.constant_inputs, &[self.logits_output], None)
                {
                    Ok(inputs) => inputs,
                    Err(err) => {
                        return Err(wrap_error(err));
                    }
                };
            self.constant_prop_inputs = Some(inputs);
        }

        if let Some(constants) = self.constant_prop_inputs.as_ref() {
            model_inputs.extend(
                constants
                    .iter()
                    .map(|(node_id, output)| (*node_id, (output).into())),
            );
        }

        // Add key-value cache from previous run.
        model_inputs.extend(
            self.kv_cache
                .iter()
                .map(|entry| (entry.input_id, entry.cache.view().into())),
        );

        // Run the model and collect outputs and updated KV cache.
        let model_outputs: Vec<NodeId> = [self.logits_output]
            .into_iter()
            .chain(self.kv_cache.iter().map(|entry| entry.output_id))
            .collect();

        let mut outputs = self
            .model
            .run(model_inputs.as_slice(), &model_outputs, None)
            .map_err(wrap_error)?;

        // Sample output token.
        let logits: NdTensor<f32, 3> = outputs.remove(0).try_into().map_err(wrap_error)?;
        let next_ids = logits
            .arg_max(-1, false /* keep_dims */)
            .map_err(wrap_error)?;
        let next_id = next_ids
            .slice::<0, _>((0, -1))
            .item()
            .map(|it| *it as u32)
            .expect("expected scalar");

        // Update the key-value cache.
        for cache_entry in self.kv_cache.iter_mut() {
            cache_entry.cache = outputs.remove(0).try_into().map_err(wrap_error)?;
        }

        // Update the token IDs for the next iteration.
        self.seq_len += self.input_ids.len() as u32;
        self.input_ids = vec![next_id];

        Ok(next_id)
    }
}

/// Output items from a [`Generator`].
pub type GeneratorItem = Result<u32, GeneratorError>;

impl<'a> Iterator for Generator<'a> {
    type Item = Result<u32, GeneratorError>;

    /// Run the model and generate the next output token.
    fn next(&mut self) -> Option<Self::Item> {
        Some(self.generate_next_token())
    }
}

/// Iterator utilities that wrap a [`Generator`] to perform common tasks such
/// as stopping generation when an end-of-text token is encountered.
pub trait GeneratorUtils: Iterator<Item = GeneratorItem> + Sized {
    /// Stop the generator when `eos_token` or an error is encountered.
    fn stop_on_token(self, eos_token: u32) -> impl Iterator<Item = GeneratorItem> {
        self.take_while(move |tok| match tok {
            Ok(tok_id) => *tok_id != eos_token,
            Err(_) => false,
        })
    }

    /// Decode the tokens to text using a tokenizer.
    fn decode<'a>(self, tokenizer: &'a Tokenizer) -> TextGenerator<'a, Self> {
        TextGenerator::wrap(self, tokenizer)
    }
}

impl<I: Iterator<Item = GeneratorItem>> GeneratorUtils for I {}

/// Wraps a [`Generator`] to decode the output token IDs from the model into
/// text using a [`Tokenizer`].
pub struct TextGenerator<'a, G: Iterator<Item = GeneratorItem>> {
    generator: G,
    tokenizer: &'a Tokenizer,
}

impl<'a, G> TextGenerator<'a, G>
where
    G: Iterator<Item = GeneratorItem>,
{
    /// Wrap a token generator and decode its outputs using `tokenizer`.
    pub fn wrap(generator: G, tokenizer: &'a Tokenizer) -> TextGenerator<'a, G> {
        TextGenerator {
            generator,
            tokenizer,
        }
    }
}

impl<'a, G: Iterator<Item = GeneratorItem>> Iterator for TextGenerator<'a, G> {
    /// The generated token string, or the error that occurred during generation.
    type Item = Result<String, GeneratorError>;

    /// Run the model repeatedly until it generates a sequence of tokens which
    /// can be decoded into a valid UTF-8 sequence.
    ///
    /// Returns `Some(Ok(text))` if successful, `Some(Err(error))` if an error
    /// occurs during generation or `None` if the end of output has been
    /// reached.
    fn next(&mut self) -> Option<Self::Item> {
        // Buffer that holds model output tokens until it forms a valid UTF-8
        // sequence.
        let mut token_buf = Vec::new();

        while let Some(token) = self.generator.next() {
            let token = match token {
                Ok(tok) => tok,
                Err(err) => return Some(Err(err)),
            };

            token_buf.push(token as usize);

            let token_strings = self.tokenizer.encoder().get_tokens(&token_buf);
            if let Ok(strings) = token_strings {
                return Some(Ok(strings.concat()));
            }
        }

        None
    }
}
