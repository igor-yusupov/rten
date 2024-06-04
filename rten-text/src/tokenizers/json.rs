//! Types for the supported subset of the `tokenizer.json` pre-trained tokenizer
//! format.

use std::collections::HashMap;

use serde::Deserialize;

#[derive(Deserialize)]
pub(crate) struct AddedToken {
    pub content: String,
}

#[derive(Deserialize)]
pub(crate) struct BertNormalizer {
    pub lowercase: bool,
    pub strip_accents: Option<bool>,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
pub(crate) enum Normalizer {
    #[serde(rename = "BertNormalizer")]
    Bert(BertNormalizer),
}

#[derive(Deserialize)]
pub(crate) struct WordPieceModel {
    /// Mapping from token text to token ID.
    pub vocab: HashMap<String, usize>,
}

#[derive(Deserialize)]
pub(crate) struct BpeModel {
    /// Mapping from token text to token ID.
    pub vocab: HashMap<String, usize>,

    /// List of `<token_a> [SPACE] <token_b>` containing tokens to merge.
    pub merges: Vec<String>,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
pub(crate) enum Model {
    #[serde(rename = "BPE")]
    Bpe(BpeModel),
    WordPiece(WordPieceModel),
}

/// Structure of the `tokenizers.json` files generated by Hugging Face
/// tokenizers [^1].
///
/// [^1]: https://github.com/huggingface/tokenizers
#[derive(Deserialize)]
pub(crate) struct TokenizerJson {
    pub added_tokens: Option<Vec<AddedToken>>,
    pub normalizer: Option<Normalizer>,
    pub model: Model,
}

/// Deserialize a `tokenizer.json` file.
pub fn from_json(json: &str) -> Result<TokenizerJson, serde_json::Error> {
    serde_json::from_str(json)
}
