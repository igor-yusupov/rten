use std::fmt::Debug;
use std::iter::zip;

use crate::linalg::{gemm, gemm_slice, Matrix};
use crate::tensor::{from_data, zero_tensor, Elements, Tensor};

mod activations;
mod binary_elementwise;
mod conv;
mod pooling;

pub use activations::{
    clip, clip_in_place, leaky_relu, leaky_relu_in_place, relu, relu_in_place, sigmoid,
    sigmoid_in_place, softmax,
};
pub use activations::{Clip, LeakyRelu, Relu, Sigmoid, Softmax};
pub use binary_elementwise::{
    add, add_in_place, div, div_in_place, mul, mul_in_place, sub, sub_in_place,
};
pub use binary_elementwise::{Add, Div, Mul, Sub};
pub use conv::{conv_2d, conv_transpose_2d};
pub use conv::{Conv2d, ConvTranspose2d};
pub use pooling::{average_pool_2d, global_average_pool, max_pool_2d};
pub use pooling::{AveragePool2d, GlobalAveragePool, MaxPool2d};

#[derive(Copy, Clone, Debug)]
pub enum Padding {
    /// Apply enough padding such that the output and input have the same size.
    ///
    /// If the required amount of padding along each dimension is even, it is
    /// divided equally between the start and the end. If it is odd, one more
    /// unit is added on the end than the start. This matches the ONNX spec
    /// for the "SAME_UPPER" value for the `auto_pad` attribute.
    Same,

    /// Apply a given amount of padding to the top, left, bottom and right of
    /// the input.
    Fixed([usize; 4]),
}

#[derive(Copy, Clone, Debug)]
pub enum DataType {
    Int32,
    Float,
}

/// Enum of the different types of input tensor that an operator can accept.
#[derive(Clone, Copy)]
pub enum Input<'a> {
    FloatTensor(&'a Tensor<f32>),
    IntTensor(&'a Tensor<i32>),
}

impl<'a> Input<'a> {
    pub fn shape(&self) -> &'a [usize] {
        match self {
            Input::FloatTensor(t) => t.shape(),
            Input::IntTensor(t) => t.shape(),
        }
    }

    pub fn as_float(&self) -> Option<&'a Tensor<f32>> {
        if let Input::FloatTensor(t) = self {
            Some(t)
        } else {
            None
        }
    }

    pub fn as_int(&self) -> Option<&'a Tensor<i32>> {
        if let Input::IntTensor(t) = self {
            Some(t)
        } else {
            None
        }
    }
}

impl<'a> From<&'a Tensor<f32>> for Input<'a> {
    fn from(t: &'a Tensor<f32>) -> Input {
        Input::FloatTensor(t)
    }
}

impl<'a> From<&'a Tensor<i32>> for Input<'a> {
    fn from(t: &'a Tensor<i32>) -> Input {
        Input::IntTensor(t)
    }
}

/// Enum of the different types of output tensor that an operator can produce.
pub enum Output {
    FloatTensor(Tensor<f32>),
    IntTensor(Tensor<i32>),
}

impl Output {
    pub fn into_int(self) -> Option<Tensor<i32>> {
        if let Output::IntTensor(t) = self {
            Some(t)
        } else {
            None
        }
    }

    pub fn as_int_ref(&self) -> Option<&Tensor<i32>> {
        if let Output::IntTensor(t) = self {
            Some(t)
        } else {
            None
        }
    }

    pub fn into_float(self) -> Option<Tensor<f32>> {
        if let Output::FloatTensor(t) = self {
            Some(t)
        } else {
            None
        }
    }

    pub fn as_float_ref(&self) -> Option<&Tensor<f32>> {
        if let Output::FloatTensor(t) = self {
            Some(t)
        } else {
            None
        }
    }
}

impl From<Tensor<f32>> for Output {
    fn from(t: Tensor<f32>) -> Output {
        Output::FloatTensor(t)
    }
}

impl From<Tensor<i32>> for Output {
    fn from(t: Tensor<i32>) -> Output {
        Output::IntTensor(t)
    }
}

#[derive(Eq, PartialEq, Debug)]
pub enum OpError {
    /// An operator was called with input tensors of a type that is not
    /// supported by the operator.
    UnsupportedInputType,

    /// The shapes of input tensors are not compatible with each other or with
    /// the operator's attributes.
    IncompatibleInputShapes(&'static str),

    /// Two operator inputs which should have the same type, have different types.
    IncompatibleInputTypes(&'static str),

    /// The operator was called with fewer inputs than expected.
    MissingInputs,

    /// An input has a value that is incorrect.
    InvalidValue(&'static str),

    /// An input or attribute has a value that is not currently supported.
    UnsupportedValue(&'static str),
}

/// An Operator is a computation step in a graph.
pub trait Operator: Debug {
    /// Return a display name for the operator.
    fn name(&self) -> &str;

    /// Execute the operator with the inputs.
    fn run(&self, input: &[Input]) -> Result<Output, OpError>;

    /// Return true if this operator supports in-place execution via
    /// `run_in_place`.
    ///
    /// In-place execution writes outputs to an existing tensor rather than
    /// allocating a new tensor. This can speed up execution by reducing the
    /// number of allocations during execution of a computation graph.
    fn can_run_in_place(&self) -> bool {
        false
    }

    /// Execute this operator in-place on an existing tensor.
    ///
    /// `input` is the first input, which the implementation may modify and
    /// return as the output. `other` are the remaining inputs.
    ///
    /// The default implementation just returns the input without modifying it.
    fn run_in_place(&self, input: Output, _other: &[Input]) -> Result<Output, OpError> {
        Ok(input)
    }
}

/// Enum of all the built-in operators
#[cfg(test)]
pub enum OpType {
    Add,
    AveragePool2d(AveragePool2d),
    BatchNormalization(BatchNormalization),
    Cast(Cast),
    Clip(Clip),
    Concat(Concat),
    ConstantOfShape(ConstantOfShape),
    Conv2d(Conv2d),
    ConvTranspose2d(ConvTranspose2d),
    Div,
    Gather(Gather),
    Gemm(Gemm),
    GlobalAveragePool,
    Identity,
    LeakyRelu(LeakyRelu),
    MatMul,
    MaxPool2d(MaxPool2d),
    Mul,
    Pad,
    Relu,
    Reshape,
    Shape,
    Sigmoid,
    Slice,
    Softmax(Softmax),
    Squeeze(Squeeze),
    Sub,
    Transpose(Transpose),
    Unsqueeze(Unsqueeze),
}

/// Extract a required float tensor input from `inputs`, or return an error.
pub fn get_input_as_float<'a>(
    inputs: &'a [Input],
    index: usize,
) -> Result<&'a Tensor<f32>, OpError> {
    inputs
        .get(index)
        .ok_or(OpError::MissingInputs)
        .and_then(|input| input.as_float().ok_or(OpError::UnsupportedInputType))
}

/// Extract an optional float tensor input from `inputs`, or return an error.
pub fn get_optional_input_as_float<'a>(
    inputs: &'a [Input],
    index: usize,
) -> Result<Option<&'a Tensor<f32>>, OpError> {
    inputs
        .get(index)
        .map(|input| input.as_float().ok_or(OpError::UnsupportedInputType))
        .transpose()
}

/// Extract a required int tensor input from `inputs`, or return an error.
pub fn get_input_as_int<'a>(inputs: &'a [Input], index: usize) -> Result<&'a Tensor<i32>, OpError> {
    inputs
        .get(index)
        .ok_or(OpError::MissingInputs)
        .and_then(|input| input.as_int().ok_or(OpError::UnsupportedInputType))
}

/// Extract an optional int tensor input from `inputs`, or return an error.
pub fn get_optional_input_as_int<'a>(
    inputs: &'a [Input],
    index: usize,
) -> Result<Option<&'a Tensor<i32>>, OpError> {
    inputs
        .get(index)
        .map(|input| input.as_int().ok_or(OpError::UnsupportedInputType))
        .transpose()
}

/// Perform in-place batch normalization on the NCHW tensor `out`.
///
/// See https://github.com/onnx/onnx/blob/main/docs/Operators.md#batchnormalization
pub fn batch_norm_in_place(
    out: &mut Tensor,
    scale: &Tensor,
    bias: &Tensor,
    mean: &Tensor,
    var: &Tensor,
    epsilon: f32,
) {
    let [batch, chans, in_h, in_w] = out.dims();
    for n in 0..batch {
        for c in 0..chans {
            let chan_mean = mean[[c]];
            let chan_var = var[[c]];
            let chan_scale = scale[[c]];
            let chan_bias = bias[[c]];

            let mut out_view = out.unchecked_view_mut([n, c, 0, 0]);

            // The batch norm formula, from the ONNX spec, is:
            //
            // Y = (X - input_mean) / sqrt(input_var + epsilon) * scale + bias
            //
            // It has been rewritten here to simplify the inner loop below.
            let scaled_std_dev_reciprocal = chan_scale / (chan_var + epsilon).sqrt();

            for y in 0..in_h {
                for x in 0..in_w {
                    let el = &mut out_view[[y, x]];
                    *el = (*el - chan_mean) * scaled_std_dev_reciprocal + chan_bias;
                }
            }
        }
    }
}

/// Perform batch normalization on the NCHW tensor `input`.
///
/// See https://github.com/onnx/onnx/blob/main/docs/Operators.md#batchnormalization
pub fn batch_norm(
    input: &Tensor,
    scale: &Tensor,
    bias: &Tensor,
    mean: &Tensor,
    var: &Tensor,
    epsilon: f32,
) -> Tensor {
    let mut output = input.clone();
    batch_norm_in_place(&mut output, scale, bias, mean, var, epsilon);
    output
}

#[derive(Debug)]
pub struct BatchNormalization {
    pub epsilon: f32,
}

impl Operator for BatchNormalization {
    fn name(&self) -> &str {
        "BatchNormalization"
    }

    fn run(&self, inputs: &[Input]) -> Result<Output, OpError> {
        let input = get_input_as_float(inputs, 0)?;
        let scale = get_input_as_float(inputs, 1)?;
        let bias = get_input_as_float(inputs, 2)?;
        let mean = get_input_as_float(inputs, 3)?;
        let var = get_input_as_float(inputs, 4)?;
        Ok(batch_norm(input, scale, bias, mean, var, self.epsilon).into())
    }

    fn can_run_in_place(&self) -> bool {
        true
    }

    fn run_in_place(&self, input: Output, other: &[Input]) -> Result<Output, OpError> {
        let mut output = input.into_float().ok_or(OpError::UnsupportedInputType)?;
        let scale = get_input_as_float(other, 0)?;
        let bias = get_input_as_float(other, 1)?;
        let mean = get_input_as_float(other, 2)?;
        let var = get_input_as_float(other, 3)?;

        batch_norm_in_place(&mut output, scale, bias, mean, var, self.epsilon);

        Ok(output.into())
    }
}

#[derive(Debug)]
pub struct Cast {
    pub to: DataType,
}

impl Operator for Cast {
    fn name(&self) -> &str {
        "Cast"
    }

    fn run(&self, inputs: &[Input]) -> Result<Output, OpError> {
        let input = inputs.get(0).ok_or(OpError::MissingInputs)?;
        match input {
            Input::IntTensor(t) => match self.to {
                DataType::Int32 => Ok((*t).clone().into()),
                DataType::Float => Ok(t.map(|x| x as f32).into()),
            },
            Input::FloatTensor(t) => match self.to {
                DataType::Int32 => Ok(t.map(|x| x as i32).into()),
                DataType::Float => Ok((*t).clone().into()),
            },
        }
    }

    fn can_run_in_place(&self) -> bool {
        true
    }

    fn run_in_place(&self, input: Output, _: &[Input]) -> Result<Output, OpError> {
        match (input, self.to) {
            (Output::IntTensor(t), DataType::Int32) => Ok(t.into()),
            (Output::IntTensor(t), _) => self.run(&[Input::IntTensor(&t)]),
            (Output::FloatTensor(t), DataType::Float) => Ok(t.into()),
            (Output::FloatTensor(t), _) => self.run(&[Input::FloatTensor(&t)]),
        }
    }
}

#[derive(Debug)]
pub struct ConstantOfShape {
    pub value: i32,
}

impl Operator for ConstantOfShape {
    fn name(&self) -> &str {
        "ConstantOfShape"
    }

    fn run(&self, inputs: &[Input]) -> Result<Output, OpError> {
        let input = get_input_as_int(inputs, 0)?;
        let shape: Vec<_> = input.elements().map(|el| el as usize).collect();
        let len = shape.iter().product();
        Ok(from_data(shape, vec![self.value; len]).into())
    }
}

/// Gather elements from `input` specified by `indices`.
///
/// See https://onnx.ai/onnx/operators/onnx__Gather.html. Per the ONNX spec this
/// is very similar to `numpy.take`. See
/// https://numpy.org/doc/stable/reference/generated/numpy.take.html for
/// additional explanation.
pub fn gather<T: Copy + Default>(
    input: &Tensor<T>,
    axis: usize,
    indices: &Tensor<i32>,
) -> Result<Tensor<T>, OpError> {
    if axis >= input.ndim() {
        return Err(OpError::InvalidValue("`axis` is out of range"));
    }
    for index in indices.elements() {
        if index < 0 || index >= input.shape()[axis] as i32 {
            return Err(OpError::InvalidValue("Entry in `indices` is out of range"));
        }
    }

    let out_shape = [
        &input.shape()[0..axis],
        indices.shape(),
        &input.shape()[axis + 1..],
    ]
    .concat();
    let mut output = zero_tensor::<T>(&out_shape);
    let mut out_index_iter = output.indices();
    let mut in_index = vec![0; input.ndim()];

    while let Some(out_index) = out_index_iter.next() {
        if out_index.is_empty() {
            // If the output index is empty, this means we are indexing a
            // 1D vector with a scalar.
            in_index[axis] = indices.item().unwrap_or(0) as usize;
        } else {
            for dim in 0..out_index.len() {
                if dim < axis {
                    in_index[dim] = out_index[dim];
                } else if dim == axis {
                    let idx = &out_index[dim..dim + indices.ndim()];
                    in_index[dim] = indices[idx] as usize;
                } else if dim >= axis + indices.ndim() {
                    in_index[dim + 1 - indices.ndim()] = out_index[dim];
                }
            }
        }
        output[out_index] = input[&in_index[..]];
    }

    Ok(output)
}

#[derive(Debug)]
pub struct Gather {
    pub axis: usize,
}

impl Operator for Gather {
    fn name(&self) -> &str {
        "Gather"
    }

    fn run(&self, inputs: &[Input]) -> Result<Output, OpError> {
        let input = inputs.get(0).ok_or(OpError::MissingInputs)?;
        let indices = get_input_as_int(inputs, 1)?;
        match input {
            Input::IntTensor(input) => gather(input, self.axis, indices).map(|t| t.into()),
            Input::FloatTensor(input) => gather(input, self.axis, indices).map(|t| t.into()),
        }
    }
}

#[derive(Debug)]
pub struct Gemm {
    pub alpha: f32,
    pub beta: f32,
    pub transpose_a: bool,
    pub transpose_b: bool,
}

/// Compute the General Matrix Multiplication (GEMM) `c = alpha * (ab) + beta * c`.
///
/// If `transpose_a` or `transpose_b` are set, the `a` and `b` inputs
/// respectively are transposed before multiplying them.
///
/// nb. This is named `gemm_op` to avoid confusion with `linalg::gemm`.
pub fn gemm_op(
    a: &Tensor,
    b: &Tensor,
    c: Option<&Tensor>,
    alpha: f32,
    beta: f32,
    transpose_a: bool,
    transpose_b: bool,
) -> Result<Tensor, OpError> {
    if alpha != 1.0 {
        return Err(OpError::UnsupportedValue(
            "Gemm only supports `alpha` value of 1.0",
        ));
    }
    if beta != 0.0 && beta != 1.0 {
        return Err(OpError::UnsupportedValue(
            "Gemm only supports `beta` values of 0.0 and 1.0",
        ));
    }

    let (a_rows, a_cols, a_row_stride, a_col_stride) = if transpose_a {
        (a.shape()[1], a.shape()[0], a.stride(1), a.stride(0))
    } else {
        (a.shape()[0], a.shape()[1], a.stride(0), a.stride(1))
    };
    let (b_rows, b_cols, b_row_stride, b_col_stride) = if transpose_b {
        (b.shape()[1], b.shape()[0], b.stride(1), b.stride(0))
    } else {
        (b.shape()[0], b.shape()[1], b.stride(0), b.stride(1))
    };

    let out_shape = &[a_rows, b_cols][..];
    let mut output = if c.is_some() && beta == 1.0 {
        let out_data = c.unwrap().broadcast_elements(out_shape).collect();
        from_data(out_shape.into(), out_data)
    } else {
        zero_tensor(out_shape)
    };

    let out_row_stride = output.stride(0);

    gemm_slice(
        output.data_mut(),
        out_row_stride,
        Matrix {
            data: a.data(),
            rows: a_rows,
            cols: a_cols,
            row_stride: a_row_stride,
            col_stride: a_col_stride,
        },
        Matrix {
            data: b.data(),
            rows: b_rows,
            cols: b_cols,
            row_stride: b_row_stride,
            col_stride: b_col_stride,
        },
    );

    Ok(output)
}

impl Operator for Gemm {
    fn name(&self) -> &str {
        "Gemm"
    }

    fn run(&self, inputs: &[Input]) -> Result<Output, OpError> {
        let a = get_input_as_float(inputs, 0)?;
        let b = get_input_as_float(inputs, 1)?;
        let c = get_optional_input_as_float(inputs, 2)?;
        gemm_op(
            a,
            b,
            c,
            self.alpha,
            self.beta,
            self.transpose_a,
            self.transpose_b,
        )
        .map(|t| t.into())
    }
}

#[derive(Debug)]
pub struct Identity {}

impl Operator for Identity {
    fn name(&self) -> &str {
        "Identity"
    }

    fn run(&self, inputs: &[Input]) -> Result<Output, OpError> {
        let input = inputs.get(0).ok_or(OpError::MissingInputs)?;
        match input {
            Input::IntTensor(t) => Ok((*t).clone().into()),
            Input::FloatTensor(t) => Ok((*t).clone().into()),
        }
    }

    fn can_run_in_place(&self) -> bool {
        true
    }

    fn run_in_place(&self, input: Output, _: &[Input]) -> Result<Output, OpError> {
        Ok(input)
    }
}

pub fn matmul(a: &Tensor, b: &Tensor) -> Result<Tensor, OpError> {
    let [a_rows, a_cols] = a.dims();
    let [b_rows, b_cols] = b.dims();

    if a_cols != b_rows {
        return Err(OpError::IncompatibleInputShapes(
            "Columns of first matrix does not match rows of second matrix",
        ));
    }

    let mut output = zero_tensor(&[a_rows, b_cols]);
    gemm(&mut output, a, b);

    Ok(output)
}

#[derive(Debug)]
pub struct MatMul {}

impl Operator for MatMul {
    fn name(&self) -> &str {
        "MatMul"
    }

    fn run(&self, inputs: &[Input]) -> Result<Output, OpError> {
        let a = get_input_as_float(inputs, 0)?;
        let b = get_input_as_float(inputs, 1)?;
        matmul(a, b).map(|t| t.into())
    }
}

pub fn reshape<T: Copy>(input: &Tensor<T>, shape: &Tensor<i32>) -> Result<Tensor<T>, OpError> {
    // If exactly one of the new shape's dimensions is -1, infer the size
    // from the input length and the sizes of the other dimensions.
    let mut unspecified_dim = None;
    let mut specified_dims_size = 1;
    for (dim, size) in shape.elements().enumerate() {
        if size < -1 {
            return Err(OpError::InvalidValue("Invalid dimension size in shape"));
        } else if size != -1 {
            specified_dims_size *= size as usize;
        } else if unspecified_dim.is_some() {
            return Err(OpError::InvalidValue(
                "Multiple dimensions in new shape set to -1",
            ));
        } else {
            unspecified_dim = Some(dim);
        }
    }
    let (unspecified_dim_size, remainder) = match input.len() {
        0 => (0, 0),
        _ => (
            input.len() / specified_dims_size,
            input.len() % specified_dims_size,
        ),
    };
    if remainder != 0 {
        return Err(OpError::InvalidValue(
            "Input length must be a multiple of specified dimensions",
        ));
    }

    let complete_shape: Vec<_> = shape
        .elements()
        .map(|size| match size {
            -1 => unspecified_dim_size,
            valid => valid as usize,
        })
        .collect();

    Ok(input.clone_with_shape(&complete_shape))
}

#[derive(Debug)]
pub struct Reshape {}
impl Operator for Reshape {
    fn name(&self) -> &str {
        "Reshape"
    }

    fn run(&self, inputs: &[Input]) -> Result<Output, OpError> {
        let input = inputs.get(0).ok_or(OpError::MissingInputs)?;
        let shape = get_input_as_int(inputs, 1)?;
        match input {
            Input::IntTensor(t) => reshape(t, shape).map(|t| t.into()),
            Input::FloatTensor(t) => reshape(t, shape).map(|t| t.into()),
        }
    }

    fn can_run_in_place(&self) -> bool {
        // The ability to reshape in place depends on input and target types.
        // If the planned inputs were passed to this method, we could do an
        // in-place reshape if the inputs/targets were compatible.
        false
    }
}

#[derive(Debug)]
pub struct Shape {}

impl Operator for Shape {
    fn name(&self) -> &str {
        "Shape"
    }

    fn run(&self, inputs: &[Input]) -> Result<Output, OpError> {
        let input = inputs.get(0).ok_or(OpError::MissingInputs)?;
        let shape = from_data(
            vec![input.shape().len()],
            input.shape().iter().map(|&el| el as i32).collect(),
        );
        Ok(shape.into())
    }
}

pub fn concat<'a, T: Copy>(inputs: &'a [&Tensor<T>], dim: usize) -> Result<Tensor<T>, OpError> {
    let first_shape = inputs[0].shape();
    if dim >= first_shape.len() {
        return Err(OpError::InvalidValue("dim is larger than input rank"));
    }

    for other in &inputs[1..] {
        let other_shape = other.shape();
        if other_shape.len() != first_shape.len() {
            return Err(OpError::IncompatibleInputShapes(
                "Tensors must have the same number of dimensions",
            ));
        }
        for d in 0..first_shape.len() {
            if d != dim && first_shape[d] != other_shape[d] {
                return Err(OpError::IncompatibleInputShapes(
                    "Dimensions must be the same except for concat dim",
                ));
            }
        }
    }

    let mut out_shape: Vec<_> = first_shape.into();
    for other in &inputs[1..] {
        out_shape[dim] += other.shape()[dim];
    }
    let mut out_data = Vec::with_capacity(out_shape.iter().product());

    struct ConcatIter<'a, T: Copy> {
        elements: Elements<'a, T>,
        chunk_size: usize,
    }

    let mut input_iters: Vec<ConcatIter<'_, T>> = inputs
        .iter()
        .map(|tensor| ConcatIter {
            elements: tensor.elements(),
            chunk_size: tensor.shape()[dim..].iter().product(),
        })
        .collect();

    while input_iters.iter().any(|it| it.elements.len() > 0) {
        for iter in input_iters.iter_mut() {
            out_data.extend(iter.elements.by_ref().take(iter.chunk_size));
        }
    }

    Ok(from_data(out_shape, out_data))
}

#[derive(Debug)]
pub struct Concat {
    pub dim: usize,
}

impl Operator for Concat {
    fn name(&self) -> &str {
        "Concat"
    }

    /// Run `concat` operator with `[a, b]` inputs.
    fn run(&self, inputs: &[Input]) -> Result<Output, OpError> {
        let first = inputs.get(0).ok_or(OpError::MissingInputs)?;

        match first {
            Input::FloatTensor(_) => {
                let typed_inputs: Vec<_> = inputs
                    .iter()
                    .map(|in_| {
                        in_.as_float().ok_or(Err::<&Tensor<f32>, OpError>(
                            OpError::IncompatibleInputTypes("Concat inputs must have same type"),
                        ))
                    })
                    .flatten()
                    .collect();
                concat(&typed_inputs, self.dim).map(|t| t.into())
            }
            Input::IntTensor(_) => {
                let typed_inputs: Vec<_> = inputs
                    .iter()
                    .map(|in_| {
                        in_.as_int().ok_or(Err::<&Tensor<i32>, OpError>(
                            OpError::IncompatibleInputTypes("Concat inputs must have same type"),
                        ))
                    })
                    .flatten()
                    .collect();
                concat(&typed_inputs, self.dim).map(|t| t.into())
            }
        }
    }
}

pub fn pad<T: Copy>(
    input: &Tensor<T>,
    padding: &Tensor<i32>,
    const_val: T,
) -> Result<Tensor<T>, OpError> {
    if padding.ndim() != 1 || padding.shape()[0] != input.ndim() * 2 {
        return Err(OpError::InvalidValue(
            "padding should be vector of length 2 * input dimensions",
        ));
    }
    if !padding.elements().all(|x| x >= 0) {
        return Err(OpError::InvalidValue("Pad only supports positive pads"));
    }

    let out_shape: Vec<_> = input
        .shape()
        .iter()
        .enumerate()
        .map(|(i, size)| {
            let start_pad = padding[[i]] as usize;
            let end_pad = padding[[input.ndim() + i]] as usize;
            start_pad + size + end_pad
        })
        .collect();
    let out_len = out_shape.iter().product();

    let mut output = from_data(out_shape, vec![const_val; out_len]);
    let mut in_iter = input.indices();
    let mut out_index = vec![0; output.shape().len()];

    while let Some(in_index) = in_iter.next() {
        out_index.copy_from_slice(&in_index);
        for i in 0..out_index.len() {
            out_index[i] += padding[[i]] as usize;
        }
        output[&out_index[..]] = input[in_index];
    }

    Ok(output)
}

fn extract_scalar<T: Copy>(x: &Tensor<T>) -> Result<T, OpError> {
    if let Some(scalar) = x.item() {
        Ok(scalar)
    } else {
        Err(OpError::InvalidValue("Expected scalar value"))
    }
}

fn extract_scalar_int(x: Input) -> Result<i32, OpError> {
    if let Input::IntTensor(val) = x {
        extract_scalar(val)
    } else {
        Err(OpError::IncompatibleInputTypes("Expected int input"))
    }
}

fn extract_scalar_float(x: Input) -> Result<f32, OpError> {
    if let Input::FloatTensor(val) = x {
        extract_scalar(val)
    } else {
        Err(OpError::IncompatibleInputTypes("Expected int input"))
    }
}

#[derive(Debug)]
pub struct Pad {}

impl Operator for Pad {
    fn name(&self) -> &str {
        "Pad"
    }

    fn run(&self, inputs: &[Input]) -> Result<Output, OpError> {
        let input = inputs.get(0).ok_or(OpError::MissingInputs)?;
        let pads = get_input_as_int(inputs, 1)?;
        let const_val = inputs.get(2);
        let axes = get_optional_input_as_int(inputs, 3)?;

        if let Some(_) = axes {
            return Err(OpError::UnsupportedValue(
                "Pad operator does not yet support `axes` input",
            ));
        }

        match input {
            Input::IntTensor(t) => {
                let const_val = const_val.map(|&v| extract_scalar_int(v)).transpose()?;
                pad(t, pads, const_val.unwrap_or(0)).map(|t| t.into())
            }
            Input::FloatTensor(t) => {
                let const_val = const_val.map(|&v| extract_scalar_float(v)).transpose()?;
                pad(t, pads, const_val.unwrap_or(0.0)).map(|t| t.into())
            }
        }
    }
}

fn slice_ranges(
    input_shape: &[usize],
    starts: &Tensor<i32>,
    ends: &Tensor<i32>,
    axes: Option<&Tensor<i32>>,
) -> Vec<(usize, usize, i32)> {
    let mut ranges: Vec<(usize, usize, i32)> = input_shape
        .iter()
        .map(|dim_size| (0, *dim_size, 1))
        .collect();
    for (i, (start, end)) in zip(starts.elements(), ends.elements()).enumerate() {
        let axis = if let Some(axes) = axes {
            axes[[i]] as usize
        } else {
            i
        };
        ranges[axis] = (start as usize, end as usize, 1);
    }
    ranges
}

/// Return a copy of a tensor which only retains a subset of a given dimension.
pub fn slice<T: Copy>(
    input: &Tensor<T>,
    starts: &Tensor<i32>,
    ends: &Tensor<i32>,
    axes: Option<&Tensor<i32>>,
) -> Tensor<T> {
    let ranges = slice_ranges(input.shape(), starts, ends, axes);
    let sliced_data = input.slice_elements(&ranges).collect();
    let sliced_shape = ranges
        .iter()
        .map(|(start, end, _step)| end - start)
        .collect();
    from_data(sliced_shape, sliced_data)
}

/// Clip the dimensions of the input tensor specified by `axes` to the ranges
/// given by `starts` and `ends`. If `axes` is
/// not set, dimensions
pub fn slice_in_place<T: Copy>(
    input: &mut Tensor<T>,
    starts: &Tensor<i32>,
    ends: &Tensor<i32>,
    axes: Option<&Tensor<i32>>,
) {
    let ranges = slice_ranges(input.shape(), starts, ends, axes);
    for (dim, (start, end, _step)) in ranges.iter().copied().enumerate() {
        input.clip_dim(dim, start, end);
    }
}

#[derive(Debug)]
pub struct Slice {}

impl Operator for Slice {
    fn name(&self) -> &str {
        "Slice"
    }

    /// Run `slice` operator with `[input, starts, ends, axes]` inputs.
    fn run(&self, inputs: &[Input]) -> Result<Output, OpError> {
        let input = inputs.get(0).ok_or(OpError::MissingInputs)?;
        let starts = get_input_as_int(inputs, 1)?;
        let ends = get_input_as_int(inputs, 2)?;
        let axes = get_optional_input_as_int(inputs, 3)?;
        let result = match input {
            Input::FloatTensor(input) => slice(input, starts, ends, axes).into(),
            Input::IntTensor(input) => slice(input, starts, ends, axes).into(),
        };
        Ok(result)
    }

    fn can_run_in_place(&self) -> bool {
        true
    }

    fn run_in_place(&self, input: Output, other: &[Input]) -> Result<Output, OpError> {
        let mut output = input.into_float().unwrap();
        let starts = other[0].as_int().unwrap();
        let ends = other[1].as_int().unwrap();
        let axes = other.get(2).map(|t| t.as_int().unwrap());
        slice_in_place(&mut output, starts, ends, axes);
        Ok(output.into())
    }
}

pub fn squeeze_in_place<T: Copy>(input: &mut Tensor<T>, axes: Option<&[usize]>) {
    let new_shape: Vec<_> = input
        .shape()
        .iter()
        .enumerate()
        .filter(|(dim, &size)| {
            if let Some(axes) = axes {
                let keep_axis = !axes.contains(dim);
                // TODO - Turn this into a result
                assert!(
                    keep_axis || size == 1,
                    "Can only remove dimensions of size 1"
                );
                keep_axis
            } else {
                size > 1
            }
        })
        .map(|(_, &size)| size)
        .collect();
    input.reshape(&new_shape);
}

pub fn squeeze<T: Copy>(input: &Tensor<T>, axes: Option<&[usize]>) -> Tensor<T> {
    let mut output = input.clone();
    squeeze_in_place(&mut output, axes);
    output
}

#[derive(Debug)]
pub struct Squeeze {
    pub axes: Option<Vec<usize>>,
}

impl Operator for Squeeze {
    fn name(&self) -> &str {
        "Squeeze"
    }

    fn run(&self, inputs: &[Input]) -> Result<Output, OpError> {
        let input = inputs.get(0).ok_or(OpError::MissingInputs)?;
        let axes = self.axes.as_ref().map(|a| &a[..]);
        let result = match input {
            Input::FloatTensor(t) => squeeze(t, axes).into(),
            Input::IntTensor(t) => squeeze(t, axes).into(),
        };
        Ok(result)
    }

    fn can_run_in_place(&self) -> bool {
        true
    }

    fn run_in_place(&self, input: Output, _: &[Input]) -> Result<Output, OpError> {
        let axes = self.axes.as_ref().map(|a| &a[..]);
        let result = match input {
            Output::FloatTensor(mut t) => {
                squeeze_in_place(&mut t, axes);
                t.into()
            }
            Output::IntTensor(mut t) => {
                squeeze_in_place(&mut t, axes);
                t.into()
            }
        };
        Ok(result)
    }
}

pub fn transpose<T: Copy>(input: &Tensor<T>, permutation: Option<&[usize]>) -> Tensor<T> {
    let mut transposed = input.clone();
    match permutation {
        Some(order) => transposed.permute(order),
        None => {
            let reversed: Vec<usize> = (0..transposed.shape().len()).rev().collect();
            transposed.permute(&reversed);
        }
    };
    transposed
}

#[derive(Debug)]
pub struct Transpose {
    /// The order of the transposed dimensions. If ommitted, the dimensions
    /// are reversed.
    pub perm: Option<Vec<usize>>,
}

impl Operator for Transpose {
    fn name(&self) -> &str {
        "Transpose"
    }

    fn run(&self, inputs: &[Input]) -> Result<Output, OpError> {
        let input = inputs.get(0).ok_or(OpError::MissingInputs)?;
        let perm_slice = self.perm.as_deref();
        let result = match input {
            Input::FloatTensor(input) => transpose(input, perm_slice).into(),
            Input::IntTensor(input) => transpose(input, perm_slice).into(),
        };
        Ok(result)
    }
}

pub fn unsqueeze<T: Copy>(input: &Tensor<T>, axes: &[usize]) -> Tensor<T> {
    let mut new_shape: Vec<_> = input.shape().to_vec();
    let mut sorted_axes: Vec<_> = axes.iter().collect();
    sorted_axes.sort();
    for &axis in sorted_axes {
        new_shape.insert(axis, 1);
    }
    input.clone_with_shape(&new_shape)
}

#[derive(Debug)]
pub struct Unsqueeze {
    pub axes: Vec<usize>,
}

impl Operator for Unsqueeze {
    fn name(&self) -> &str {
        "Unsqueeze"
    }

    fn run(&self, inputs: &[Input]) -> Result<Output, OpError> {
        let input = inputs.get(0).ok_or(OpError::MissingInputs)?;
        let result = match input {
            Input::FloatTensor(input) => unsqueeze(input, &self.axes).into(),
            Input::IntTensor(input) => unsqueeze(input, &self.axes).into(),
        };
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use crate::linalg::gemm;
    use crate::ops::{
        batch_norm, batch_norm_in_place, concat, gather, gemm_op, matmul, pad, reshape, slice,
        slice_in_place, squeeze, squeeze_in_place, transpose, unsqueeze, Cast, ConstantOfShape,
        DataType, Identity, Input, OpError, Operator, Pad, Reshape, Shape,
    };
    use crate::rng::XorShiftRNG;
    use crate::tensor::{from_data, from_scalar, from_vec, random_tensor, zero_tensor, Tensor};
    use crate::test_util::expect_equal;

    #[test]
    fn test_batch_norm() -> Result<(), String> {
        let input = from_data(vec![1, 2, 1, 1], vec![1.0, 2.0]);
        let scale = from_data(vec![2], vec![3.0, 3.0]);
        let bias = from_data(vec![2], vec![0.1, 0.2]);
        let mean = from_data(vec![2], vec![0.5, -0.5]);
        let var = from_data(vec![2], vec![1.0, 2.0]);

        let epsilon = 1e-5 as f32;

        let y1 = (input[[0, 0, 0, 0]] - mean[[0]]) / (var[[0]] + epsilon).sqrt() * scale[[0]]
            + bias[[0]];
        let y2 = (input[[0, 1, 0, 0]] - mean[[1]]) / (var[[1]] + epsilon).sqrt() * scale[[1]]
            + bias[[1]];
        let expected = from_data(vec![1, 2, 1, 1], vec![y1, y2]);
        let result = batch_norm(&input, &scale, &bias, &mean, &var, epsilon);

        expect_equal(&result, &expected)
    }

    #[test]
    fn test_batch_norm_in_place() -> Result<(), String> {
        let mut input = from_data(vec![1, 2, 1, 1], vec![1.0, 2.0]);
        let scale = from_data(vec![2], vec![3.0, 3.0]);
        let bias = from_data(vec![2], vec![0.1, 0.2]);
        let mean = from_data(vec![2], vec![0.5, -0.5]);
        let var = from_data(vec![2], vec![1.0, 2.0]);

        let epsilon = 1e-5 as f32;

        let y1 = (input[[0, 0, 0, 0]] - mean[[0]]) / (var[[0]] + epsilon).sqrt() * scale[[0]]
            + bias[[0]];
        let y2 = (input[[0, 1, 0, 0]] - mean[[1]]) / (var[[1]] + epsilon).sqrt() * scale[[1]]
            + bias[[1]];
        let expected = from_data(vec![1, 2, 1, 1], vec![y1, y2]);

        batch_norm_in_place(&mut input, &scale, &bias, &mean, &var, epsilon);

        expect_equal(&input, &expected)
    }

    #[test]
    fn test_cast() -> Result<(), String> {
        let int_input = from_vec(vec![1, 2, 3]);
        let float_input = from_vec(vec![1.0, 2.0, 3.0]);

        // No-op cast from int32 => int32
        let cast_to_int = Cast {
            to: DataType::Int32,
        };
        let result = cast_to_int
            .run(&[Input::IntTensor(&int_input)])
            .unwrap()
            .into_int()
            .unwrap();

        // Flooring cast from float => int32
        assert_eq!(result, int_input);
        let result = cast_to_int
            .run(&[Input::FloatTensor(&float_input)])
            .unwrap()
            .into_int()
            .unwrap();
        assert_eq!(&result, &int_input);

        // No-op cast from float => float
        let cast_to_float = Cast {
            to: DataType::Float,
        };
        let result = cast_to_float
            .run(&[Input::FloatTensor(&float_input)])
            .unwrap()
            .into_float()
            .unwrap();
        expect_equal(&result, &float_input)?;

        // Cast from int32 => float
        let result = cast_to_float
            .run(&[Input::IntTensor(&int_input)])
            .unwrap()
            .into_float()
            .unwrap();
        expect_equal(&result, &float_input)
    }

    #[test]
    fn test_cast_out_of_range() -> Result<(), String> {
        let int_input = from_vec(vec![i32::MIN, i32::MAX]);

        // Out-of-range cast from int => float. This will simply lose some
        // significant digits.
        let cast_to_float = Cast {
            to: DataType::Float,
        };
        let result = cast_to_float
            .run(&[(&int_input).into()])
            .unwrap()
            .into_float()
            .unwrap();
        expect_equal(&result, &from_vec(vec![-2147483600.0, 2147483600.0]))?;

        // Out-of-range cast from float => int.
        let float_input = from_vec(vec![f32::MIN, f32::MAX]);
        let cast_to_int = Cast {
            to: DataType::Int32,
        };
        let result = cast_to_int
            .run(&[(&float_input).into()])
            .unwrap()
            .into_int()
            .unwrap();
        assert_eq!(&result, &from_vec(vec![i32::MIN, i32::MAX]));

        Ok(())
    }

    #[test]
    fn test_constant_of_shape() {
        let op = ConstantOfShape { value: 42 };
        let shape = from_vec(vec![1, 5, 10]);

        let result = op
            .run(&[Input::IntTensor(&shape)])
            .unwrap()
            .into_int()
            .unwrap();

        assert_eq!(result.shape(), &[1, 5, 10]);
        assert_eq!(
            result.elements().collect::<Vec<_>>(),
            vec![42; result.shape().iter().product()]
        );
    }

    #[test]
    fn test_gather_scalar() {
        let input = from_vec(vec![1, 20, 30]);
        for i in 0..input.len() {
            let indices = from_scalar(i as i32);
            let result = gather(&input, 0, &indices).unwrap();
            assert_eq!(result.item(), Some(input[[i]]))
        }
    }

    #[test]
    fn test_gather() -> Result<(), String> {
        // Test case shrunk down from a small BERT model where `gather` is used
        // to lookup up embeddings.
        let mut rng = XorShiftRNG::new(1234);
        let input = random_tensor(&[128, 10], &mut rng);
        let indices = from_data(vec![2, 2], vec![2, 5, 8, 50]);
        let result = gather(&input, 0, &indices).unwrap();
        assert_eq!(result.shape(), &[2, 2, 10]);

        // Test case #1 from ONNX spec.
        let input = from_data(vec![3, 2], vec![1.0, 1.2, 2.3, 3.4, 4.5, 5.7]);
        let indices = from_data(vec![2, 2], vec![0, 1, 1, 2]);
        let expected = from_data(vec![2, 2, 2], vec![1.0, 1.2, 2.3, 3.4, 2.3, 3.4, 4.5, 5.7]);
        let result = gather(&input, 0, &indices).unwrap();
        expect_equal(&result, &expected)?;

        // Test case #2 from ONNX spec.
        let input = from_data(
            vec![3, 3],
            vec![1.0, 1.2, 1.9, 2.3, 3.4, 3.9, 4.5, 5.7, 5.9],
        );
        let indices = from_data(vec![1, 2], vec![0, 2]);
        let expected = from_data(vec![3, 1, 2], vec![1.0, 1.9, 2.3, 3.9, 4.5, 5.9]);
        let result = gather(&input, 1, &indices).unwrap();
        expect_equal(&result, &expected)
    }

    #[test]
    fn test_gather_invalid_inputs() {
        let mut rng = XorShiftRNG::new(1234);
        let input = random_tensor(&[128, 10], &mut rng);
        let indices = from_data(vec![2, 2], vec![2, 5, 8, 50]);
        let result = gather(&input, 5, &indices);
        assert_eq!(
            result.err(),
            Some(OpError::InvalidValue("`axis` is out of range"))
        );

        let indices = from_data(vec![2, 2], vec![2, 5, 8, 130]);
        let result = gather(&input, 0, &indices);
        assert_eq!(
            result.err(),
            Some(OpError::InvalidValue("Entry in `indices` is out of range"))
        );
    }

    #[test]
    fn test_gemm_op() -> Result<(), String> {
        let mut rng = XorShiftRNG::new(1234);
        let a = random_tensor(&[3, 10], &mut rng);
        let b = random_tensor(&[10, 8], &mut rng);

        let mut expected = zero_tensor(&[3, 8]);
        gemm(&mut expected, &a, &b);

        let result = gemm_op(&a, &b, None, 1.0, 1.0, false, false).unwrap();

        expect_equal(&result, &expected)
    }

    #[test]
    fn test_gemm_op_transposed() -> Result<(), String> {
        let mut rng = XorShiftRNG::new(1234);
        let a = random_tensor(&[10, 3], &mut rng);
        let b = random_tensor(&[8, 10], &mut rng);

        let mut a_transposed = a.clone();
        a_transposed.permute(&[1, 0]);
        let mut b_transposed = b.clone();
        b_transposed.permute(&[1, 0]);
        let mut expected = zero_tensor(&[3, 8]);
        gemm(&mut expected, &a_transposed, &b_transposed);

        let result = gemm_op(&a, &b, None, 1.0, 1.0, true, true).unwrap();

        expect_equal(&result, &expected)
    }

    #[test]
    fn test_gemm_op_adds_c() -> Result<(), String> {
        let mut rng = XorShiftRNG::new(1234);
        let a = random_tensor(&[3, 10], &mut rng);
        let b = random_tensor(&[10, 8], &mut rng);
        let c = random_tensor(&[3, 8], &mut rng);

        let mut expected = c.clone();
        gemm(&mut expected, &a, &b);

        let result = gemm_op(&a, &b, Some(&c), 1.0, 1.0, false, false).unwrap();

        expect_equal(&result, &expected)
    }

    #[test]
    fn test_identity() -> Result<(), String> {
        let id_op = Identity {};

        let int_input = from_vec(vec![1, 2, 3]);
        let result = id_op
            .run(&[Input::IntTensor(&int_input)])
            .unwrap()
            .into_int()
            .unwrap();
        assert_eq!(result, int_input);

        let float_input = from_vec(vec![1.0, 2.0, 3.0]);
        let result = id_op
            .run(&[Input::FloatTensor(&float_input)])
            .unwrap()
            .into_float()
            .unwrap();
        expect_equal(&result, &float_input)
    }

    #[test]
    fn test_matmul() -> Result<(), String> {
        let mut rng = XorShiftRNG::new(1234);
        let a = random_tensor(&[3, 10], &mut rng);
        let b = random_tensor(&[10, 8], &mut rng);

        let mut expected = zero_tensor(&[3, 8]);
        gemm(&mut expected, &a, &b);

        let result = matmul(&a, &b).unwrap();
        expect_equal(&result, &expected)
    }

    #[test]
    fn test_reshape_with_unspecified_dim() -> Result<(), String> {
        // Reshape with an unspecified (-1) dim and nonzero-length input
        let input = from_data(vec![2, 2], vec![-0.5, 0.5, 3.0, -5.5]);
        let shape = from_vec(vec![1, -1, 2]);
        let expected = input.clone_with_shape(&[1, 2, 2]);
        let result = reshape(&input, &shape).unwrap();
        expect_equal(&result, &expected)?;

        // Reshape with an unspecified (-1) dim and zero-length input
        let zero_sized_input = from_data(vec![4, 0, 1], vec![]);
        let shape = from_vec(vec![100, -1]);
        let result = reshape(&zero_sized_input, &shape).unwrap();
        let expected = zero_sized_input.clone_with_shape(&[100, 0]);
        expect_equal(&result, &expected)
    }

    #[test]
    fn test_reshape_with_multiple_unspecified_dims() {
        let input = from_data(vec![2, 2], vec![-0.5, 0.5, 3.0, -5.5]);
        let shape = from_vec(vec![1, -1, -1]);
        assert_eq!(
            reshape(&input, &shape).err(),
            Some(OpError::InvalidValue(
                "Multiple dimensions in new shape set to -1"
            ))
        );
    }

    #[test]
    fn test_reshape_with_unsolvable_unspecified_dim() {
        let input = from_data(vec![2, 2], vec![-0.5, 0.5, 3.0, -5.5]);
        let shape = from_vec(vec![5, -1]);
        assert_eq!(
            reshape(&input, &shape).err(),
            Some(OpError::InvalidValue(
                "Input length must be a multiple of specified dimensions"
            ))
        );
    }

    #[test]
    fn test_reshape_op() -> Result<(), String> {
        let input = from_data(vec![2, 2], vec![-0.5, 0.5, 3.0, -5.5]);
        let shape = from_data(vec![1], vec![4]);
        let expected = input.clone_with_shape(&[4]);

        let op = Reshape {};
        let result = op
            .run(&[(&input).into(), (&shape).into()])
            .unwrap()
            .into_float()
            .unwrap();

        expect_equal(&result, &expected)
    }

    #[test]
    fn test_concat() -> Result<(), String> {
        let a = from_data(vec![2, 2, 1], vec![0.1, 0.2, 0.3, 0.4]);
        let b = from_data(vec![2, 2, 1], vec![1.0, 2.0, 3.0, 4.0]);

        // Concatenation along the first dimension
        let expected = from_data(vec![4, 2, 1], vec![0.1, 0.2, 0.3, 0.4, 1.0, 2.0, 3.0, 4.0]);
        let result = concat(&[&a, &b], 0).unwrap();
        expect_equal(&result, &expected)?;

        // Concatenation along a non-first dimension
        let expected = from_data(vec![2, 2, 2], vec![0.1, 1.0, 0.2, 2.0, 0.3, 3.0, 0.4, 4.0]);
        let result = concat(&[&a, &b], 2).unwrap();
        expect_equal(&result, &expected)?;

        // Concatenation with one input
        let result = concat(&[&a], 0).unwrap();
        expect_equal(&result, &a)?;

        // Concatenation with more than two inputs
        let result = concat(&[&a, &b, &a], 0).unwrap();
        assert_eq!(result.shape(), &[6, 2, 1]);

        // Concatentation with some empty inputs
        let a = from_slice(&[1, 2, 3]);
        let b = from_slice(&[]);
        let c = from_slice(&[4, 5, 6]);
        let result = concat(&[&a, &b, &c], 0).unwrap();
        assert_eq!(result.shape(), &[6]);
        assert_eq!(result.data(), &[1, 2, 3, 4, 5, 6]);

        Ok(())
    }

    #[test]
    fn test_concat_invalid_inputs() {
        // Invalid `dim` attribute
        let input = from_slice(&[1, 2, 3]);
        let result = concat(&[&input, &input], 1);
        assert_eq!(
            result.err(),
            Some(OpError::InvalidValue("dim is larger than input rank"))
        );

        // Shape mismatch
        let a = zero_tensor::<f32>(&[1]);
        let b = zero_tensor::<f32>(&[1, 2]);
        let result = concat(&[&a, &b], 0);
        assert_eq!(
            result.err(),
            Some(OpError::IncompatibleInputShapes(
                "Tensors must have the same number of dimensions"
            ))
        );

        // Shape mismatch in non-`dim` dimension
        let a = zero_tensor::<f32>(&[5, 10]);
        let b = zero_tensor::<f32>(&[5, 11]);
        let result = concat(&[&a, &b], 0);
        assert_eq!(
            result.err(),
            Some(OpError::IncompatibleInputShapes(
                "Dimensions must be the same except for concat dim"
            ))
        );
    }

    #[test]
    fn test_pad() -> Result<(), String> {
        // Same padding around each edge.
        let input = from_data(vec![2, 2], vec![1.0, 2.0, 3.0, 4.0]);
        let expected = from_data(
            vec![4, 4],
            vec![
                0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 0.0, 0.0, 3.0, 4.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            ],
        );
        let const_pads = from_slice(&[1, 1, 1, 1]);
        let result = pad(&input, &const_pads, 0.0).unwrap();
        expect_equal(&result, &expected)?;

        // Zero padding (no-op)
        let zero_pads = from_slice(&[0, 0, 0, 0]);
        let result = pad(&input, &zero_pads, 0.0).unwrap();
        expect_equal(&result, &input)?;

        // Un-even padding
        let input = from_data(vec![1, 2, 2], vec![1, 2, 3, 4]);
        let pads = from_slice(&[0, 0, 0, 0, 1, 0]);
        let result = pad(&input, &pads, 0).unwrap();
        assert_eq!(result.shape(), &[1, 3, 2]);
        assert_eq!(result.data(), &[1, 2, 3, 4, 0, 0]);

        Ok(())
    }

    #[test]
    fn test_pad_constant_val() -> Result<(), String> {
        let input = from_data(vec![2, 2], vec![1.0, 2.0, 3.0, 4.0]);
        let expected = from_data(
            vec![4, 4],
            vec![
                9., 9., 9., 9., 9., 1., 2., 9., 9., 3., 4., 9., 9., 9., 9., 9.,
            ],
        );
        let const_pads = from_slice(&[1, 1, 1, 1]);
        let result = pad(&input, &const_pads, 9.).unwrap();
        expect_equal(&result, &expected)
    }

    #[test]
    fn test_pad_op() -> Result<(), String> {
        let input = from_data(vec![2, 2], vec![1.0, 2.0, 3.0, 4.0]);
        let pads = from_slice(&[1, 1, 1, 1]);
        let expected = from_data(
            vec![4, 4],
            vec![
                0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 0.0, 0.0, 3.0, 4.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            ],
        );

        let op = Pad {};
        let result = op
            .run(&[(&input).into(), (&pads).into()])
            .ok()
            .and_then(|r| r.into_float())
            .unwrap();
        expect_equal(&result, &expected)?;

        Ok(())
    }

    #[test]
    fn test_pad_invalid_inputs() {
        let input = from_data(vec![2, 2], vec![1.0, 2.0, 3.0, 4.0]);
        let op = Pad {};

        // Wrong padding vector length.
        let invalid_pads = from_slice(&[1]);
        let result = op.run(&[(&input).into(), (&invalid_pads).into()]);
        assert_eq!(
            result.err(),
            Some(OpError::InvalidValue(
                "padding should be vector of length 2 * input dimensions"
            ))
        );

        // Unsupported padding amounts.
        let invalid_pads = from_slice(&[1, 1, 1, -1]);
        let result = op.run(&[(&input).into(), (&invalid_pads).into()]);
        assert_eq!(
            result.err(),
            Some(OpError::InvalidValue("Pad only supports positive pads"))
        );

        // Wrong constant value type.
        let invalid_pads = from_slice(&[1, 1, 1, -1]);
        let const_int = from_scalar(1);
        let result = op.run(&[(&input).into(), (&invalid_pads).into(), (&const_int).into()]);
        assert_eq!(
            result.err(),
            Some(OpError::IncompatibleInputTypes("Expected int input"))
        );

        // Constant value not a scalar.
        let invalid_pads = from_slice(&[1, 1, 1, -1]);
        let int_vec = from_slice(&[1.0, 2.0]);
        let result = op.run(&[(&input).into(), (&invalid_pads).into(), (&int_vec).into()]);
        assert_eq!(
            result.err(),
            Some(OpError::InvalidValue("Expected scalar value"))
        );
    }

    #[test]
    fn test_transpose() -> Result<(), String> {
        let mut rng = XorShiftRNG::new(5678);
        let input = random_tensor(&[10, 20], &mut rng);

        let mut reversed = input.clone();
        reversed.permute(&[1, 0]);

        // With no explicit permutation given, the axes should be reversed.
        let result = transpose(&input, None);
        expect_equal(&result, &reversed)?;

        // With a no-op permutation given, the output should be unchanged.
        let result = transpose(&input, Some(&[0, 1]));
        expect_equal(&result, &input)?;

        // With a transposed permutation given, the axes should be reversed.
        let result = transpose(&input, Some(&[1, 0]));
        expect_equal(&result, &reversed)
    }

    #[test]
    fn test_shape() {
        let op = Shape {};

        // Float input
        let input = from_data(vec![1, 1, 2, 2], vec![1.0, 2.0, 3.0, 4.0]);
        let result = op.run(&[(&input).into()]).unwrap().into_int().unwrap();
        assert_eq!(result.shape(), &[4]);
        assert_eq!(result.data(), &[1, 1, 2, 2]);

        // Int input
        let input = from_data(vec![1, 1, 2, 2], vec![1, 2, 3, 4]);
        let result = op.run(&[(&input).into()]).unwrap().into_int().unwrap();
        assert_eq!(result.shape(), &[4]);
        assert_eq!(result.data(), &[1, 1, 2, 2]);
    }

    fn from_slice<T: Copy>(data: &[T]) -> Tensor<T> {
        from_data(vec![data.len()], data.into())
    }

    #[test]
    fn test_slice_in_place() {
        let mut rng = XorShiftRNG::new(5678);
        let mut input = random_tensor(&[2, 2, 5, 3], &mut rng);

        let starts = from_slice(&[2]);
        let ends = from_slice(&[4]);
        let axes = from_slice(&[2]);

        slice_in_place(&mut input, &starts, &ends, Some(&axes));

        assert_eq!(
            input.shape(),
            vec![2, 2, ends[[0]] as usize - starts[[0]] as usize, 3]
        );
    }

    #[test]
    fn test_slice_not_first_dim() {
        let mut rng = XorShiftRNG::new(5678);
        let input = random_tensor(&[2, 2, 5, 3], &mut rng);

        let starts = from_slice(&[2]);
        let ends = from_slice(&[4]);
        let axes = from_slice(&[2]);

        let sliced = slice(&input, &starts, &ends, Some(&axes));
        let shape = sliced.shape();

        assert_eq!(
            sliced.shape(),
            vec![2, 2, ends[[0]] as usize - starts[[0]] as usize, 3]
        );
        assert_eq!(sliced.len(), shape.iter().fold(1, |len, x| len * x));

        for w in 0..shape[0] {
            for x in 0..shape[1] {
                for y in 0..shape[2] {
                    for z in 0..shape[3] {
                        assert_eq!(
                            sliced[[w, x, y, z]],
                            input[[w, x, y + starts[[0]] as usize, z]]
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn test_slice_first_dim() {
        let mut rng = XorShiftRNG::new(5678);
        let input = random_tensor(&[5, 2, 5, 3], &mut rng);

        let starts = from_slice(&[2]);
        let ends = from_slice(&[4]);
        let axes = from_slice(&[0]);

        let sliced = slice(&input, &starts, &ends, Some(&axes));
        let shape = sliced.shape();

        assert_eq!(
            shape,
            vec![ends[[0]] as usize - starts[[0]] as usize, 2, 5, 3]
        );
        assert_eq!(sliced.len(), shape.iter().fold(1, |len, x| len * x));

        for w in 0..shape[0] {
            for x in 0..shape[1] {
                for y in 0..shape[2] {
                    for z in 0..shape[3] {
                        assert_eq!(
                            sliced[[w, x, y, z]],
                            input[[w + starts[[0]] as usize, x, y, z]]
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn test_slice_noop() {
        let mut rng = XorShiftRNG::new(5678);
        let input = random_tensor(&[5, 2, 5, 3], &mut rng);

        for dim in 0..input.shape().len() {
            let dim_size = input.shape()[dim] as i32;

            let starts = from_slice(&[0]);
            let ends = from_slice(&[dim_size]);
            let axes = from_slice(&[dim as i32]);

            let sliced = slice(&input, &starts, &ends, Some(&axes));
            assert_eq!(sliced.shape(), input.shape());
            assert_eq!(sliced.data(), input.data());
        }
    }

    #[test]
    fn test_squeeze() -> Result<(), String> {
        let mut rng = XorShiftRNG::new(5678);
        let input = random_tensor(&[1, 5, 5, 1], &mut rng);
        let mut expected = input.clone();

        // Remove all 1-size axes.
        expected.reshape(&[5, 5]);
        let result = squeeze(&input, None);
        expect_equal(&result, &expected)?;

        // Remove final 1-size axis.
        expected.reshape(&[1, 5, 5]);
        let result = squeeze(&input, Some(&[3]));
        expect_equal(&result, &expected)?;

        // Remove first 1-size axis.
        expected.reshape(&[5, 5, 1]);
        let result = squeeze(&input, Some(&[0]));
        expect_equal(&result, &expected)
    }

    #[test]
    fn test_squeeze_in_place() -> Result<(), String> {
        let mut rng = XorShiftRNG::new(5678);
        let mut input = random_tensor(&[1, 1, 5, 5], &mut rng);

        let mut expected = input.clone();
        expected.reshape(&[5, 5]);

        squeeze_in_place(&mut input, None);

        expect_equal(&input, &expected)
    }

    #[test]
    fn test_unsqueeze() {
        let mut rng = XorShiftRNG::new(5678);
        let input = random_tensor(&[3, 4, 5], &mut rng);

        // Unsqueeze with axes in increasing order
        let output = unsqueeze(&input, &[0, 4]);
        assert_eq!(output.shape(), &[1, 3, 4, 5, 1]);

        // Unsqueeze with axes in decreasing order
        let output = unsqueeze(&input, &[4, 0]);
        assert_eq!(output.shape(), &[1, 3, 4, 5, 1]);

        // Unsqueeze a scalar into a 1-item vec
        let scalar = from_scalar(2.0);
        let output = unsqueeze(&scalar, &[0]);
        assert_eq!(output.shape(), &[1]);
        assert_eq!(output.data(), &[2.0]);
    }
}
