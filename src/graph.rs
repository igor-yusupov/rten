use std::collections::{HashMap, HashSet};
use std::time::Instant;

use crate::ops::Operator;
use crate::tensor::Tensor;

struct OperatorNode {
    inputs: Vec<NodeId>,
    output: NodeId,
    operator: Box<dyn Operator>,
}

enum Node {
    Operator(OperatorNode),
    Constant(Tensor),
    Value,
}

pub type NodeId = usize;

pub struct Graph {
    nodes: Vec<Node>,
}

pub struct RunOptions {
    /// Whether to log operator timing as the graph executes
    pub timing: bool,
}

impl Default for RunOptions {
    fn default() -> Self {
        RunOptions { timing: false }
    }
}

impl Graph {
    /// Create a new empty dataflow graph.
    pub fn new() -> Graph {
        Graph { nodes: Vec::new() }
    }

    /// Add an operator node to the graph.
    ///
    /// `inputs` specifies which other nodes in the graph should be used as
    /// inputs to this operation when the graph is executed. These other nodes
    /// can be inputs, constants (for weights and biases) or outputs of other
    /// operators.
    ///
    /// Returns the ID of the added operator's output node.
    pub fn add_op(&mut self, op: Box<dyn Operator>, inputs: &[NodeId]) -> NodeId {
        let output_id = self.add_value();
        self.nodes.push(Node::Operator(OperatorNode {
            inputs: Vec::from(inputs),
            output: output_id,
            operator: op,
        }));
        output_id
    }

    /// Add a constant node to the graph.
    ///
    /// Returns the ID of the added node.
    pub fn add_constant(&mut self, value: Tensor) -> NodeId {
        self.nodes.push(Node::Constant(value));
        self.nodes.len() - 1
    }

    /// Add a value node to the graph.
    ///
    /// This serves as a placeholder for a value which is available only when
    /// the graph is executed, such as an input or operator output.
    ///
    /// Returns the ID of the added node.
    pub fn add_value(&mut self) -> NodeId {
        self.nodes.push(Node::Value);
        self.nodes.len() - 1
    }

    /// Compute a set of output values given a set of inputs, using the
    /// processing steps and constant values defined by the graph.
    pub fn run(
        &self,
        inputs: &[(NodeId, &Tensor)],
        outputs: &[NodeId],
        opts: Option<RunOptions>,
    ) -> Vec<Tensor> {
        let plan = self.create_plan(inputs, outputs);
        let opts = opts.unwrap_or_default();

        // Collect operator inputs
        let mut values: HashMap<NodeId, &Tensor> = inputs.iter().map(|x| *x).collect();
        for (node_id, node) in self.nodes.iter().enumerate() {
            if let Node::Constant(tensor) = node {
                values.insert(node_id, tensor);
            }
        }

        // Execute the plan
        let mut temp_values: HashMap<NodeId, Tensor> = HashMap::new();
        for (op_node_id, op_node) in plan.iter() {
            let mut op_inputs = Vec::new();
            for node_id in op_node.inputs.iter() {
                if let Some(value) = values.get(&node_id) {
                    op_inputs.push(*value);
                } else if let Some(value) = temp_values.get(&node_id) {
                    op_inputs.push(value);
                } else {
                    // If this is reached, there was a bug in plan creation.
                    panic!(
                        "Invalid plan did not produce input value {} for operator {}",
                        node_id, op_node_id
                    );
                }
            }

            let op_start = if opts.timing {
                Some(Instant::now())
            } else {
                None
            };

            let output = op_node.operator.run(&op_inputs[..]);

            if let Some(start) = op_start {
                let input_shapes: Vec<_> = op_inputs.iter().map(|x| x.shape()).collect();
                let op_elapsed = start.elapsed().as_millis();
                println!(
                    "#{} {:?} with {:?} / {}ms",
                    op_node_id, op_node.operator, input_shapes, op_elapsed
                );
            }

            temp_values.insert(op_node.output, output);
            // TODO - Remove temporary inputs that are no longer needed
        }

        // Return the requested outputs
        outputs
            .iter()
            .map(|output_id| {
                if let Some(&value) = values.get(output_id) {
                    value.clone()
                } else if let Some(value) = temp_values.remove(output_id) {
                    value
                } else {
                    unreachable!()
                }
            })
            .collect()
    }

    /// Create an execution plan for a sequence of computation steps that begin
    /// with `inputs` and eventually produces `outputs`.
    ///
    /// Any node IDs in `outputs` which reference constant or input values are
    /// omitted from the plan.
    fn create_plan(
        &self,
        inputs: &[(NodeId, &Tensor)],
        outputs: &[NodeId],
    ) -> Vec<(NodeId, &OperatorNode)> {
        // Map of output node to source operator
        let operator_nodes: HashMap<NodeId, &OperatorNode> = self
            .nodes
            .iter()
            .filter_map(|node| match node {
                Node::Operator(op_node) => Some((op_node.output, op_node)),
                _ => None,
            })
            .collect();

        // Set of values that are available after executing the plan
        let mut resolved_values: HashSet<NodeId> =
            inputs.iter().map(|(node_id, _)| *node_id).collect();
        for (node_id, node) in self.nodes.iter().enumerate() {
            if let Node::Constant(_) = node {
                resolved_values.insert(node_id);
            }
        }

        // Build an execution plan via a depth first traversal of the graph
        // starting at the output nodes. A helper struct is used as recursive
        // closures are not supported in Rust.
        struct PlanBuilder<'a> {
            resolved_values: HashSet<NodeId>,
            plan: Vec<(NodeId, &'a OperatorNode)>,
            operator_nodes: HashMap<NodeId, &'a OperatorNode>,
        }
        impl<'a> PlanBuilder<'a> {
            fn visit(&mut self, node_id: NodeId, op_node: &'a OperatorNode) {
                for input in op_node.inputs.iter() {
                    if self.resolved_values.contains(input) {
                        continue;
                    }
                    if let Some(input_op_node) = self.operator_nodes.get(&input) {
                        self.visit(*input, input_op_node);
                    } else {
                        panic!("Unable to generate execution plan. Missing value {}", input)
                    }
                }
                self.resolved_values.insert(node_id);
                self.plan.push((node_id, &op_node));
            }

            fn plan(mut self, outputs: &[NodeId]) -> Vec<(NodeId, &'a OperatorNode)> {
                for output_id in outputs.iter() {
                    if let Some(op_node) = self.operator_nodes.get(&output_id) {
                        self.visit(*output_id, op_node);
                    } else if !self.resolved_values.contains(output_id) {
                        panic!(
                            "Unable to generate execution plan. Missing value {}",
                            output_id
                        )
                    }
                }
                self.plan
            }
        }

        let builder = PlanBuilder {
            resolved_values,
            plan: Vec::new(),
            operator_nodes,
        };
        builder.plan(outputs)
    }
}

#[cfg(test)]
mod tests {
    use crate::graph::Graph;
    use crate::ops::{Concat, Conv2d, Operator, ReLU};
    use crate::tensor::{from_data, Tensor};

    /// Check that the shapes of two tensors are equal and that their contents
    /// are approximately equal.
    fn expect_equal(x: &Tensor, y: &Tensor) -> Result<(), String> {
        if x.shape() != y.shape() {
            return Err(format!(
                "Tensors have different shapes. {:?} vs. {:?}",
                x.shape(),
                y.shape()
            ));
        }

        let eps = 0.001;
        for i in 0..x.len() {
            let xi = x.data()[i];
            let yi = y.data()[i];

            if (xi - yi).abs() > eps {
                return Err(format!(
                    "Tensor values differ at index {}: {} vs {}",
                    i, xi, yi
                ));
            }
        }

        return Ok(());
    }

    // Test of a very simple graph with a typical structure (one input, one
    // output, Conv2d + ReLU operation).
    #[test]
    fn test_graph_run() -> Result<(), String> {
        let mut g = Graph::new();

        let weights = from_data(
            vec![1, 1, 3, 3],
            vec![
                0.3230, 0.7632, 0.4616, 0.8837, 0.5898, 0.3424, 0.2101, 0.7821, 0.6861,
            ],
        );
        let weights_id = g.add_constant(weights);
        let input_id = g.add_value();

        let conv_out = g.add_op(
            Box::new(Conv2d {
                padding: (1, 1),
                groups: 1,
            }),
            &[input_id, weights_id],
        );
        let relu_out = g.add_op(Box::new(ReLU {}), &[conv_out]);

        let input = from_data(
            vec![1, 1, 3, 3],
            vec![
                0.5946, 0.8249, 0.0448, 0.9552, 0.2041, 0.2501, 0.2693, 0.1007, 0.8862,
            ],
        );

        let results = g.run(&[(input_id, &input)], &[relu_out], None);

        let expected = from_data(
            vec![1, 1, 3, 3],
            vec![
                1.5202, 1.5592, 0.9939, 1.7475, 2.6358, 1.3428, 1.0165, 1.1806, 0.8685,
            ],
        );
        assert_eq!(results.len(), 1);
        expect_equal(&results[0], &expected)
    }

    #[derive(Debug)]
    struct AddOne {}
    impl Operator for AddOne {
        fn run(&self, inputs: &[&Tensor]) -> Tensor {
            let input = inputs[0];
            let output_data = input.data().iter().map(|x| x + 1.0).collect();
            from_data(input.shape().into(), output_data)
        }
    }

    #[test]
    fn test_graph_planning_order() -> Result<(), String> {
        let mut g = Graph::new();

        let input_id = g.add_value();

        let op_a = g.add_op(Box::new(AddOne {}), &[input_id]);
        let op_b = g.add_op(Box::new(AddOne {}), &[op_a]);

        // op_c has both op_a and op_b as inputs. Since op_b depends on op_a,
        // execution must run op_a, then op_b, then op_c.
        let op_c = g.add_op(Box::new(Concat { dim: 0 }), &[op_a, op_b]);

        // op_d is the same as op_c, but input order is reversed
        let op_d = g.add_op(Box::new(Concat { dim: 0 }), &[op_b, op_a]);

        let input = from_data(vec![1], vec![1.]);

        let results = g.run(&[(input_id, &input)], &[op_c], None);
        let expected = from_data(vec![2], vec![2., 3.]);
        expect_equal(&results[0], &expected)?;

        let results = g.run(&[(input_id, &input)], &[op_d], None);
        let expected = from_data(vec![2], vec![3., 2.]);
        expect_equal(&results[0], &expected)
    }

    #[test]
    fn test_graph_many_steps() -> Result<(), String> {
        let mut g = Graph::new();

        let input = from_data(vec![5], vec![1., 2., 3., 4., 5.]);
        let input_id = g.add_value();

        let mut prev_output = input_id;
        for _ in 0..100 {
            prev_output = g.add_op(Box::new(AddOne {}), &[prev_output]);
        }

        let results = g.run(&[(input_id, &input)], &[prev_output], None);

        let expected = from_data(vec![5], vec![101., 102., 103., 104., 105.]);
        expect_equal(&results[0], &expected)
    }

    #[test]
    fn test_noop_graph() -> Result<(), String> {
        let mut g = Graph::new();

        let input = from_data(vec![5], vec![1., 2., 3., 4., 5.]);
        let input_id = g.add_value();

        let results = g.run(&[(input_id, &input)], &[input_id], None);

        expect_equal(&results[0], &input)
    }

    #[test]
    fn test_constant_graph() -> Result<(), String> {
        let mut g = Graph::new();

        let value = from_data(vec![5], vec![1., 2., 3., 4., 5.]);
        let const_id = g.add_constant(value.clone());

        let results = g.run(&[], &[const_id], None);

        expect_equal(&results[0], &value)
    }

    #[test]
    fn test_no_outputs() {
        let g = Graph::new();
        let results = g.run(&[], &[], None);
        assert_eq!(results.len(), 0);
    }

    #[test]
    #[should_panic(expected = "Unable to generate execution plan. Missing value 123")]
    fn test_panic_if_invalid_output() {
        let g = Graph::new();
        g.run(&[], &[123], None);
    }

    #[test]
    #[should_panic(expected = "Unable to generate execution plan. Missing value 42")]
    fn test_panic_if_missing_operator_input() {
        let mut g = Graph::new();
        let output = g.add_op(Box::new(ReLU {}), &[42]);
        g.run(&[], &[output], None);
    }
}
