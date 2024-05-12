use std::iter::zip;

use rten_tensor::prelude::*;
use rten_tensor::{NdTensorView, SliceItem, SliceRange, Tensor, TensorView};

use crate::ops::{resolve_axis, Input, InputList, IntoOpResult, OpError, Operator, Output};
use crate::static_dims;
use crate::tensor_pool::TensorPool;

/// Compute the effective starts, ends and steps for each input dimension in
/// a Slice operation.
///
/// See https://onnx.ai/onnx/operators/onnx__Slice.html.
fn slice_ranges(
    input_shape: &[usize],
    starts: &NdTensorView<i32, 1>,
    ends: &NdTensorView<i32, 1>,
    axes: Option<&NdTensorView<i32, 1>>,
    steps: Option<&NdTensorView<i32, 1>>,
) -> Result<Vec<SliceRange>, OpError> {
    // FIXME: Verify that `starts`, `ends`, `axes` and `steps` have compatible
    // lengths.

    if let Some(steps) = steps {
        for &step in steps.iter() {
            if step == 0 {
                return Err(OpError::InvalidValue("steps must be non-zero"));
            }
        }
    }

    let mut ranges: Vec<SliceRange> = input_shape
        .iter()
        .map(|dim_size| SliceRange::new(0, Some(*dim_size as isize), 1))
        .collect();
    for (i, (start, end)) in zip(starts.iter(), ends.iter()).enumerate() {
        let axis = if let Some(axes) = axes {
            resolve_axis(input_shape.len(), axes[[i]] as isize)?
        } else {
            i
        };

        let step = steps.map(|s| s[[i]]).unwrap_or(1);
        ranges[axis] = SliceRange::new(*start as isize, Some(*end as isize), step as isize);
    }
    Ok(ranges)
}

/// Return a copy of a tensor which only retains a subset of a given dimension.
pub fn slice<T: Copy>(
    pool: &TensorPool,
    input: TensorView<T>,
    starts: &NdTensorView<i32, 1>,
    ends: &NdTensorView<i32, 1>,
    axes: Option<&NdTensorView<i32, 1>>,
    steps: Option<&NdTensorView<i32, 1>>,
) -> Result<Tensor<T>, OpError> {
    let ranges = slice_ranges(input.shape(), starts, ends, axes, steps)?;
    let items: Vec<_> = ranges.iter().map(|r| SliceItem::Range(*r)).collect();
    Ok(input.slice_copy_in(pool, items.as_slice()))
}

/// Clip the dimensions of the input tensor specified by `axes` to the ranges
/// given by `starts` and `ends`.
pub fn slice_in_place<T: Copy>(
    input: &mut Tensor<T>,
    starts: &NdTensorView<i32, 1>,
    ends: &NdTensorView<i32, 1>,
    axes: Option<&NdTensorView<i32, 1>>,
) -> Result<(), OpError> {
    let ranges = slice_ranges(input.shape(), starts, ends, axes, None)?;
    for (dim, range) in ranges.iter().enumerate() {
        let dim_size = input.size(dim);
        input.clip_dim(dim, range.resolve_clamped(dim_size));
    }
    Ok(())
}

#[derive(Debug)]
pub struct Slice {}

impl Operator for Slice {
    fn name(&self) -> &str {
        "Slice"
    }

    fn run(&self, pool: &TensorPool, inputs: InputList) -> Result<Vec<Output>, OpError> {
        let input = inputs.require(0)?;

        let starts = inputs.require_as::<i32>(1)?;
        let starts = static_dims!(starts, 1)?;

        let ends = inputs.require_as::<i32>(2)?;
        let ends = static_dims!(ends, 1)?;

        let axes = inputs
            .get_as::<i32>(3)?
            .map(|axes| static_dims!(axes, 1))
            .transpose()?;

        let steps = inputs
            .get_as::<i32>(4)?
            .map(|steps| static_dims!(steps, 1))
            .transpose()?;

        let result: Result<Output, OpError> = match input {
            Input::FloatTensor(input) => {
                slice(pool, input, &starts, &ends, axes.as_ref(), steps.as_ref()).map(|t| t.into())
            }
            Input::IntTensor(input) => {
                slice(pool, input, &starts, &ends, axes.as_ref(), steps.as_ref()).map(|t| t.into())
            }
        };
        result.into_op_result()
    }

    fn can_run_in_place(&self) -> bool {
        true
    }

    fn run_in_place(
        &self,
        pool: &TensorPool,
        input: Output,
        other: InputList,
    ) -> Result<Output, OpError> {
        let starts = other.require_as::<i32>(0)?;
        let starts = static_dims!(starts, 1)?;

        let ends = other.require_as::<i32>(1)?;
        let ends = static_dims!(ends, 1)?;

        let axes = other
            .get_as::<i32>(2)?
            .map(|axes| static_dims!(axes, 1))
            .transpose()?;
        let steps = other
            .get_as::<i32>(3)?
            .map(|steps| static_dims!(steps, 1))
            .transpose()?;

        // Fall back to copying if non-default steps are given.
        if let Some(steps) = steps {
            if steps.iter().any(|step| *step != 1) {
                let mut inputs: Vec<_> = vec![(&input).into()];
                inputs.extend(other.iter());
                return self
                    .run(pool, InputList::from(&inputs))
                    .map(|mut outputs| outputs.remove(0));
            }
        }

        match input {
            Output::IntTensor(mut output) => {
                slice_in_place(&mut output, &starts, &ends, axes.as_ref())?;
                Ok(output.into())
            }
            Output::FloatTensor(mut output) => {
                slice_in_place(&mut output, &starts, &ends, axes.as_ref())?;
                Ok(output.into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use rten_tensor::prelude::*;
    use rten_tensor::rng::XorShiftRng;
    use rten_tensor::test_util::expect_equal;
    use rten_tensor::Tensor;

    use crate::ops::tests::new_pool;
    use crate::ops::{slice, slice_in_place};

    fn from_slice<T: Copy>(data: &[T]) -> Tensor<T> {
        Tensor::from_data(&[data.len()], data.to_vec())
    }

    #[test]
    fn test_slice_in_place() {
        // Slice with +ve and in-bounds endpoints.
        let mut rng = XorShiftRng::new(5678);
        let mut input = Tensor::rand(&[2, 2, 5, 3], &mut rng);

        let starts = &[2];
        let ends = &[4];
        let axes = &[2];

        slice_in_place(&mut input, &starts.into(), &ends.into(), Some(&axes.into())).unwrap();

        assert_eq!(
            input.shape(),
            vec![2, 2, ends[0] as usize - starts[0] as usize, 3]
        );

        // Slice with -ve endpoints.
        let mut input = Tensor::from_vec((0..10).collect());
        let starts = &[-9];
        let ends = &[-6];
        slice_in_place(&mut input, &starts.into(), &ends.into(), None).unwrap();
        assert_eq!(input.to_vec(), &[1, 2, 3]);

        // Slice with out-of-bounds end.
        let mut input = Tensor::from_vec((0..10).collect());
        let starts = &[5];
        let ends = &[20];
        slice_in_place(&mut input, &starts.into(), &ends.into(), None).unwrap();
        assert_eq!(input.to_vec(), &[5, 6, 7, 8, 9]);

        // Slice with out-of-bounds start.
        let mut input = Tensor::from_vec((0..10).collect());
        let starts = &[-20];
        let ends = &[5];
        slice_in_place(&mut input, &starts.into(), &ends.into(), None).unwrap();
        assert_eq!(input.to_vec(), &[0, 1, 2, 3, 4]);
    }

    #[test]
    fn test_slice_first_dim() {
        let pool = new_pool();
        let mut rng = XorShiftRng::new(5678);
        let input = Tensor::rand(&[5, 2, 5, 3], &mut rng);

        let starts = &[2];
        let ends = &[4];
        let axes = &[0];

        let sliced = slice(
            &pool,
            input.view(),
            &starts.into(),
            &ends.into(),
            Some(&axes.into()),
            None,
        )
        .unwrap();
        let shape = sliced.shape();

        assert_eq!(shape, vec![ends[0] as usize - starts[0] as usize, 2, 5, 3]);
        assert_eq!(sliced.len(), shape.iter().fold(1, |len, x| len * x));

        for w in 0..shape[0] {
            for x in 0..shape[1] {
                for y in 0..shape[2] {
                    for z in 0..shape[3] {
                        assert_eq!(
                            sliced[[w, x, y, z]],
                            input[[w + starts[0] as usize, x, y, z]]
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn test_slice_inner_dim() {
        let pool = new_pool();
        let mut rng = XorShiftRng::new(5678);
        let input = Tensor::rand(&[2, 2, 5, 3], &mut rng);

        let starts = &[2];
        let ends = &[4];
        let axes = &[2];

        let sliced = slice(
            &pool,
            input.view(),
            &starts.into(),
            &ends.into(),
            Some(&axes.into()),
            None,
        )
        .unwrap();
        let shape = sliced.shape();

        assert_eq!(
            sliced.shape(),
            vec![2, 2, ends[0] as usize - starts[0] as usize, 3]
        );
        assert_eq!(sliced.len(), shape.iter().fold(1, |len, x| len * x));

        for w in 0..shape[0] {
            for x in 0..shape[1] {
                for y in 0..shape[2] {
                    for z in 0..shape[3] {
                        assert_eq!(
                            sliced[[w, x, y, z]],
                            input[[w, x, y + starts[0] as usize, z]]
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn test_slice_noop() {
        let pool = new_pool();
        let mut rng = XorShiftRng::new(5678);
        let input = Tensor::rand(&[5, 2, 5, 3], &mut rng);

        for dim in 0..input.shape().len() {
            let dim_size = input.size(dim) as i32;

            let starts = &[0];
            let ends = &[dim_size];
            let axes = &[dim as i32];

            let sliced = slice(
                &pool,
                input.view(),
                &starts.into(),
                &ends.into(),
                Some(&axes.into()),
                None,
            )
            .unwrap();
            assert_eq!(sliced, input);
        }
    }

    #[test]
    fn test_slice_negative_axes() {
        let pool = new_pool();
        let input = Tensor::from_data(&[3, 3], vec![1, 2, 3, 4, 5, 6, 7, 8, 9]);
        let starts = &[0];
        let ends = &[2];

        let axes = &[-1];
        let sliced = slice(
            &pool,
            input.view(),
            &starts.into(),
            &ends.into(),
            Some(&axes.into()),
            None,
        )
        .unwrap();
        assert_eq!(sliced.to_vec(), &[1, 2, 4, 5, 7, 8]);

        let axes = &[-2];
        let sliced = slice(
            &pool,
            input.view(),
            &starts.into(),
            &ends.into(),
            Some(&axes.into()),
            None,
        )
        .unwrap();
        assert_eq!(sliced.to_vec(), &[1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn test_slice_negative_starts() {
        let pool = new_pool();
        let input = Tensor::from_data(&[3, 3], vec![1, 2, 3, 4, 5, 6, 7, 8, 9]);
        let axes = &[-1];
        let ends = &[2];

        let starts = &[-3];
        let sliced = slice(
            &pool,
            input.view(),
            &starts.into(),
            &ends.into(),
            Some(&axes.into()),
            None,
        )
        .unwrap();
        assert_eq!(sliced.to_vec(), &[1, 2, 4, 5, 7, 8]);

        let starts = &[-2];
        let sliced = slice(
            &pool,
            input.view(),
            &starts.into(),
            &ends.into(),
            Some(&axes.into()),
            None,
        )
        .unwrap();
        assert_eq!(sliced.to_vec(), &[2, 5, 8]);
    }

    #[test]
    fn test_slice_negative_ends() {
        let pool = new_pool();
        let input = Tensor::from_data(&[3, 3], vec![1, 2, 3, 4, 5, 6, 7, 8, 9]);
        let axes = &[-1];
        let starts = &[0];

        let ends = &[-1];
        let sliced = slice(
            &pool,
            input.view(),
            &starts.into(),
            &ends.into(),
            Some(&axes.into()),
            None,
        )
        .unwrap();
        assert_eq!(sliced.to_vec(), &[1, 2, 4, 5, 7, 8]);

        let ends = &[-2];
        let sliced = slice(
            &pool,
            input.view(),
            &starts.into(),
            &ends.into(),
            Some(&axes.into()),
            None,
        )
        .unwrap();
        assert_eq!(sliced.to_vec(), &[1, 4, 7]);
    }

    #[test]
    fn test_slice_clamps_starts_and_ends() -> Result<(), Box<dyn Error>> {
        let pool = new_pool();
        let mut rng = XorShiftRng::new(5678);
        let input = Tensor::rand(&[20, 20], &mut rng);

        // Simulate how a range without a start/end may be given in a model.
        //
        // The ONNX Slice spec does not support unbounded ranges (like
        // `array[start:]` in numpy) but instead recommends the use of INT_MAX /
        // -INT_MAX together with clamping to achieve the same result.
        let starts = &[-i32::MAX, -100];
        let ends = &[i32::MAX, 100];

        let sliced = slice(
            &pool,
            input.view(),
            &starts.into(),
            &ends.into(),
            None,
            None,
        )
        .unwrap();

        expect_equal(&sliced, &input)?;

        Ok(())
    }

    #[test]
    fn test_slice_with_step() {
        let input = from_slice(&[1, 2, 3, 4, 5]);

        struct Case<'a> {
            start: i32,
            end: i32,
            step: i32,
            expected_shape: &'a [usize],
            expected_elements: &'a [i32],
        }

        let cases = [
            // Positive step > 1
            Case {
                start: 0,
                end: 5,
                step: 2,
                expected_shape: &[3],
                expected_elements: &[1, 3, 5],
            },
            // Negative step
            Case {
                start: 5,
                end: -6,
                step: -1,
                expected_shape: &[5],
                expected_elements: &[5, 4, 3, 2, 1],
            },
            // Negative step with clamped start
            Case {
                start: 100,
                end: -6,
                step: -1,
                expected_shape: &[5],
                expected_elements: &[5, 4, 3, 2, 1],
            },
            // Negative step with clamped end
            Case {
                start: 5,
                end: -100,
                step: -1,
                expected_shape: &[5],
                expected_elements: &[5, 4, 3, 2, 1],
            },
        ];

        let pool = new_pool();
        for case in cases {
            let starts = &[case.start];
            let ends = &[case.end];
            let axes = &[0];
            let steps = &[case.step];

            let sliced = slice(
                &pool,
                input.view(),
                &starts.into(),
                &ends.into(),
                Some(&axes.into()),
                Some(&steps.into()),
            )
            .unwrap();
            assert_eq!(
                sliced,
                Tensor::from_data(case.expected_shape, case.expected_elements.to_vec())
            );
        }
    }
}
