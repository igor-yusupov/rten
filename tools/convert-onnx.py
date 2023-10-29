#!/usr/bin/env python

from argparse import ArgumentParser
import sys
from typing import Any, Callable, Literal, Optional, cast

import flatbuffers
import numpy as np
import onnx
import onnx.numpy_helper as numpy_helper
from onnx import TensorProto

import schema_generated as sg

AttributeValue = int | float | str | list[int]


EMITTED_WARNINGS: set[str] = set()


def warn_once(msg: str):
    """
    Emit a warning if not already emitted.

    This is used to reduce output noise if the same problem arises many times
    when converting a model.
    """
    if msg in EMITTED_WARNINGS:
        return
    EMITTED_WARNINGS.add(msg)
    print(msg, file=sys.stderr)


class Node:
    """Base class for all graph nodes (constants, values, operators)."""

    def __init__(self, name: str):
        self.name = name


class ConstantNode(Node):
    """
    Data for a constant graph node.

    These are used for model weights, biases etc.
    """

    shape: list[int]
    data: np.ndarray

    def __init__(self, name: str, shape: list[int], data: np.ndarray):
        super().__init__(name)
        self.shape = shape
        self.data = data

        shape_numel = np.prod(shape)
        if shape_numel != data.size:
            raise ValueError(
                f'Shape {shape} product {shape_numel} does not match data length {data.size} in node "{name}"'
            )

        # Verify that this is a data type that we'll be able to serialize later.
        match data.dtype:
            case np.float32 | np.int32:
                pass
            case _:
                dtype_name = data.dtype.name
                raise ValueError(
                    f'Tried to construct ConstantNode "{name}" with unsupported data type {dtype_name}'
                )

    def get_scalar(self):
        if self.shape != []:
            return None
        return self.data[0]


class OperatorNode(Node):
    """
    Data for an operator graph node.
    """

    # Wasnn operator name. This should match the operator name in the FlatBuffers
    # schema.
    op_type: str

    attrs: Any
    """
    Attributes object or None.

    This should be the operator-specific attributes object generated by flatc.
    eg. `sg.AveragePoolAttrsT` for the AveragePool op.
    """

    inputs: list[int | None]
    outputs: list[int | None]

    def __init__(
        self,
        name: str,
        op_type: str,
        attrs: Any,
        inputs: list[int | None],
        outputs: list[int | None],
    ):
        super().__init__(name)
        self.op_type = op_type
        self.attrs = attrs
        self.inputs = inputs
        self.outputs = outputs


class ValueNode(Node):
    """
    Data for a value placeholder graph node.

    These are used for operator inputs and outputs.

    The shape can be missing, or a mix of fixed and symbolic (unknown at model
    export time) sizes.
    """

    def __init__(self, name: str, shape: list[int | str] | None):
        super().__init__(name)

        self.shape = shape


class Graph:
    nodes: list[Node]

    inputs: list[int]
    """Indices of nodes in `nodes` that are model inputs."""

    outputs: list[int]
    """Indices of nodes in `nodes` that are model outputs."""

    def __init__(self, nodes: list[Node], inputs: list[int], outputs: list[int]):
        self.nodes = nodes
        self.inputs = inputs
        self.outputs = outputs


# Mapping of ONNX attribute types to the field on an AttributeProto which
# contains the value. Note that if you try to access the wrong field on an
# AttributeProto, you get a default value instead of an exception.
value_fields = {
    onnx.AttributeProto.FLOAT: "f",
    onnx.AttributeProto.INT: "i",
    onnx.AttributeProto.INTS: "ints",
    onnx.AttributeProto.STRING: "s",
    onnx.AttributeProto.TENSOR: "t",
}


def snake_case_to_pascal_case(s: str) -> str:
    """Transform a snake_case string to PascalCase."""
    return "".join([word[0].upper() + word[1:] for word in s.split("_")])


class ONNXOperatorReader:
    """
    Utiliy for extracting attribute and input values from an ONNX operator.

    This keeps track of which attributes have been read so that we can warn about
    any unhandled ones.
    """

    onnx_op: onnx.OperatorProto

    add_node: Callable[[Node], int]
    """
    Function that adds a new node to the graph and returns its ID.

    This is used if a new constant node has to be generated to replace an
    operator attribute.
    """

    input_indexes: list[int | None]
    """
    IDs of the operator's input nodes.

    New inputs may be generated while reading an operator if it has an attribute
    that needs to be converted to a dynamic input.
    """

    _handled_attrs: set[str]
    """Names of attributes that have been handled."""

    def __init__(
        self,
        onnx_op: onnx.OperatorProto,
        input_indexes: list[int | None],
        add_node: Callable[[Node], int],
    ):
        self.onnx_op = onnx_op

        self.add_node = add_node
        self.input_indexes = input_indexes.copy()

        self._handled_attrs = set()

    def get_attr(self, name: str, expected_type: str, default):
        """Get the value of an optional operator attribute."""

        self._handled_attrs.add(name)

        type_code = getattr(onnx.AttributeProto, expected_type.upper())
        for attr in self.onnx_op.attribute:
            if attr.name == name:
                if attr.type != type_code:
                    raise Exception(
                        f"Attribute {name} type does not match {expected_type}"
                    )
                val = getattr(attr, value_fields[type_code])

                # String attribute values are stored as bytes, so we have to decode
                # them.
                if expected_type == "string":
                    val = val.decode()

                return val
        return default

    def get_enum_attr(self, name: str, enum: Any, default: str):
        """
        Get an optional attribute whose value is an enum variant.

        The variant name is Pascal-Cased and looked up on the enum object.
        eg. `round_prefer_floor` => `RoundPreferFloor`. If the Pascal-Cased
        name matches a Python keyword, it is expected to be escaped, eg.
        `none` => `None_`.
        """
        val = self.get_attr(name, "string", default)
        pascal_case = snake_case_to_pascal_case(val)

        # Enum values that match Python keywords have a trailing underscore appended.
        escaped_pascal_case = pascal_case + "_"

        try:
            try:
                return getattr(enum, pascal_case)
            except AttributeError:
                return getattr(enum, escaped_pascal_case)
        except AttributeError:
            raise ValueError(f"Unsupported value {val} for {name} attr")

    def ignore_attr(self, name: str):
        """
        Mark an attribute as ignored.

        This is useful in cases where an attribute contains redundant information.
        """
        self._handled_attrs.add(name)

    def require_attr(self, name: str, expected_type: str):
        """Get the value of a required operator attribute."""
        val = self.get_attr(name, expected_type, default=None)
        if val is None:
            raise Exception(f"Missing required attribute {name}")
        return val

    def generate_input_from_attr(
        self, input_index: int, attr_name: str, attr_type: str
    ):
        """
        Generate a constant operator input from an attribute, if it exists.

        Some operator inputs changed from attributes to inputs in different ONNX
        releases. This function checks to see if an operator has an attribute
        and synthesizes a constant input.

        :param input_index: Index of the input that the attribute corresponds to
        :param attr_name: Name of the attribute
        :param attr_type: Expected type of the attribute
        """

        attr_val = self.get_attr(attr_name, attr_type, default=None)
        if attr_val is None:
            return

        if input_index < len(self.input_indexes):
            raise Exception(
                f'Operator has both an attribute "{attr_name}" and corresponding input at index {input_index}'
            )

        match attr_type:
            case "int":
                shape = []
                data = np.array(attr_val).astype(np.int32)

            case "float":
                shape = []
                data = np.array(attr_val).astype(np.float32)

            case "ints":
                shape = [len(attr_val)]
                data = np.array([attr_val]).astype(np.int32)
            case _:
                raise ValueError(
                    f'Unable to generate input from "{attr_name}" attribute of type "{attr_type}"'
                )

        generated_name = self.onnx_op.name + ":wasnn-" + attr_name
        const_node = ConstantNode(generated_name, shape, data)
        input_id = self.add_node(const_node)

        while len(self.input_indexes) < input_index + 1:
            self.input_indexes.append(None)
        self.input_indexes[input_index] = input_id

    def check_attr(self, name: str, expected_type, default):
        """
        Check if an operator has an unsupported non-default value for an attribute.

        If `default` is a tuple, it specifies a set of acceptable defaults.
        """

        val = self.get_attr(name, expected_type, None)
        if val is None:
            return

        if not isinstance(default, tuple):
            default = (default,)
        if val not in default:
            raise Exception(
                f"Unsupported value {val} for attribute {name}. Default is {default}"
            )

    def unhandled_attrs(self) -> list[onnx.AttributeProto]:
        """Return a list of attributes which have not been read."""
        return [
            attr
            for attr in self.onnx_op.attribute
            if attr.name not in self._handled_attrs
        ]


def check_ints_length(name: str, ints: list[int], allowed_length: int):
    """
    Check that an ints attribute has a fixed length.

    Various ONNX operators allow for a wider range of dimensions and per-axis
    values (eg. for strides, dilations, padding...) than this library currently
    supports.
    """
    if len(ints) != allowed_length:
        raise Exception(f'Attribute "{name}" must have {allowed_length} values')


def constant_node_from_onnx_initializer(
    tensor: onnx.TensorProto, op_name: Optional[str]
) -> ConstantNode:

    dims = list(tensor.dims)
    data = numpy_helper.to_array(tensor)

    match data.dtype.name:
        # Types that don't need to change
        case "float32" | "int32":
            pass

        # Int types that can be widened to int32
        case "bool" | "int8" | "int16":
            data = data.astype(np.int32)

        # Types that need to be narrowed
        case "int64":
            # Some ONNX exporters use `INT_MIN` and `INT_MAX` to represent
            # infinity in certain cases, for example slicing to the end of a
            # dimension with unknown size (see
            # https://github.com/onnx/onnx/blob/main/docs/Operators.md#slice and
            # https://github.com/pytorch/pytorch/issues/17606).
            #
            # In the case where the value is an `int64` and we are converting
            # this to an `int32` in the model, this will cause an overflow. To
            # resolve this, clamp the value to the min/max values for the
            # smaller integer type we are using.
            i32 = np.iinfo(np.int32)
            out_of_range_mask = np.logical_or(data > i32.max, data < i32.min)
            for val in data[out_of_range_mask]:
                warn_once(
                    f"Clamping out-of-range tensor value {val} to [{i32.min}, {i32.max}]"
                )
            data = data.clip(i32.min, i32.max).astype(np.int32)

        case _:
            raise ValueError(
                f"Unsupported tensor data type {data.dtype.name} for operator {op_name}"
            )

    return ConstantNode(name=tensor.name, shape=dims, data=data)


def constant_node_from_onnx_constant_op(onnx_op: onnx.OperatorProto) -> ConstantNode:
    def noop_add_node(node: Node) -> int:
        raise ValueError("Not implemented")

    if not len(onnx_op.output):
        raise Exception(f'Operator "{onnx_op.name}" has no outputs')

    output_name = onnx_op.output[0]

    tensor = ONNXOperatorReader(
        onnx_op, input_indexes=[], add_node=noop_add_node
    ).require_attr("value", "tensor")
    const_node = constant_node_from_onnx_initializer(tensor, output_name)
    const_node.name = output_name

    return const_node


def value_node_from_onnx_value(value: onnx.ValueInfoProto) -> ValueNode:
    if value.type.tensor_type.shape.dim:
        dims = [d.dim_param or d.dim_value for d in value.type.tensor_type.shape.dim]
    else:
        dims = None
    return ValueNode(name=value.name, shape=dims)


def read_pads(op_reader: ONNXOperatorReader) -> tuple[str, list[int] | None]:
    """
    Read a padding specification from an ONNX operator.
    """

    pads = None
    auto_pad = op_reader.get_attr("auto_pad", "string", "NOTSET")

    match auto_pad:
        case "SAME_UPPER" | "SAME_LOWER":
            pad_mode = "same"
            pads = []
        case "NOTSET":
            pad_mode = "fixed"
            pads = op_reader.get_attr("pads", "ints", [0, 0, 0, 0])
            if len(pads) not in [2, 4]:
                raise Exception('"padding" attribute must have 2 or 4 values')
        case other:
            raise Exception(f"Unsupported auto_pad value {other}")

    return (pad_mode, pads)


def read_strides(
    op_reader: ONNXOperatorReader,
):
    """
    Read a stride specification from an ONNX operator.
    """
    strides = op_reader.get_attr("strides", "ints", [1, 1])
    if len(strides) not in [1, 2]:
        raise Exception('"strides" attribute must have 1 or 2 values')
    return strides


def read_dilations(
    op_reader: ONNXOperatorReader,
):
    """
    Read a dilation specification from an ONNX operator.
    """
    dilations = op_reader.get_attr("dilations", "ints", [1, 1])
    if len(dilations) not in [1, 2]:
        raise Exception('"dilations" attribute must have 1 or 2 values')
    return dilations


def op_node_from_onnx_operator(
    onnx_op: onnx.OperatorProto,
    node_index_from_name: dict[str, int],
    constant_nodes: dict[str, ConstantNode],
    add_node: Callable[[Node], int],
) -> OperatorNode:
    """
    Map an ONNX operator to the equivalent operator in this library.

    See https://github.com/onnx/onnx/blob/main/docs/Operators.md for list of
    available ONNX operators and attributes for each.

    :param onnx_op: ONNX operator to convert
    :param node_index_from_name: Mapping of constant and value tensor node names
      in the graph to corresponding input names
    :param constant_nodes: Map of constant value tensor node names
    :param add_node: Function that adds a new node to the graph and returns its
      node ID. This is called if an operator attribute needs to be converted
      to a constant input.
    """
    input_indexes = []
    for input_name in onnx_op.input:
        if input_name:
            index = node_index_from_name.get(input_name)
            if index is None:
                raise Exception(
                    f'Unable to find input "{input_name}" for operator {onnx_op.name}'
                )
        else:
            # An empty input name indicates an omitted optional input. This is
            # only required in cases where at least one subsequent optional
            # input is provided. All trailing optional inputs can simply be
            # omitted.
            index = None
        input_indexes.append(index)

    output_indexes = []
    for output_name in onnx_op.output:
        index = node_index_from_name.get(output_name)
        if index is None:
            raise Exception(
                f'Unable to find output "{output_name}" for operator {onnx_op.name}'
            )
        output_indexes.append(index)

    # Operator attributes. This will be `None` for operators with no attributes,
    # or one of the `OperatorNameAttrsT` classes generated by flatc.
    attrs = None

    # Operator type name in Wasnn models. By default assume this is the same as
    # the ONNX type.
    op_type = onnx_op.op_type
    op_reader = ONNXOperatorReader(onnx_op, input_indexes, add_node)

    # Check / convert operator attributes and operator name, if different than
    # ONNX.
    match op_type:
        case "ArgMax" | "ArgMin":
            attrs = sg.ArgMaxAttrsT()
            attrs.axes = op_reader.get_attr("axis", "int", None)
            attrs.keepDims = bool(op_reader.get_attr("keepdims", "int", 1))
            op_reader.check_attr("select_last_index", "int", 0)

        case "AveragePool":
            kernel_shape = op_reader.require_attr("kernel_shape", "ints")
            check_ints_length("kernel_shape", kernel_shape, 2)
            pad_mode, pads = read_pads(op_reader)
            op_reader.check_attr("ceil_mode", "int", 0)
            op_reader.check_attr("count_include_pad", "int", 0)

            attrs = sg.AveragePoolAttrsT()
            attrs.kernelSize = kernel_shape
            if pads:
                attrs.pads = pads
            if pad_mode == "same":
                attrs.padMode = sg.PadMode.Same
            else:
                attrs.padMode = sg.PadMode.Fixed
            attrs.strides = read_strides(op_reader)

        case "BatchNormalization" | "InstanceNormalization":
            attrs = sg.BatchNormalizationAttrsT()
            attrs.epsilon = op_reader.get_attr("epsilon", "float", 1e-5)

        case "Cast":
            attrs = sg.CastAttrsT()
            to = op_reader.get_attr("to", "int", TensorProto.DataType.FLOAT)
            match to:
                case TensorProto.DataType.FLOAT:
                    attrs.to = sg.DataType.Float
                case TensorProto.DataType.BOOL | TensorProto.DataType.INT32 | TensorProto.DataType.INT64:
                    attrs.to = sg.DataType.Int32
                case _:
                    raise Exception(f"Unsupported target type for cast {to}")

        case "Clip":
            op_reader.generate_input_from_attr(1, "min", "float")
            op_reader.generate_input_from_attr(2, "max", "float")

        case "Concat":
            attrs = sg.ConcatAttrsT()
            attrs.axis = op_reader.require_attr("axis", "int")

        case "ConstantOfShape":
            tensor = op_reader.require_attr("value", "tensor")
            const_node = constant_node_from_onnx_initializer(tensor, onnx_op.name)

            if len(const_node.data) != 1:
                raise Exception(
                    "Expected ConstantOfShape value to be a 1-element tensor"
                )

            if const_node.data.dtype == np.float32:
                scalar_type = sg.Scalar.FloatScalar
                scalar = sg.FloatScalarT()
                scalar.value = const_node.data.item()
            elif const_node.data.dtype == np.int32:
                scalar_type = sg.Scalar.IntScalar
                scalar = sg.IntScalarT()
                scalar.value = const_node.data.item()
            else:
                raise ValueError(
                    f"Unsupported value type {const_node.data.dtype.name} for ConstantOfShape"
                )

            attrs = sg.ConstantOfShapeAttrsT()
            attrs.valueType = scalar_type
            attrs.value = scalar

        case "Conv":
            attrs = sg.ConvAttrsT()
            attrs.dilations = read_dilations(op_reader)
            attrs.groups = op_reader.get_attr("group", "int", 1)

            pad_mode, pads = read_pads(op_reader)
            if pad_mode == "same":
                attrs.padMode = sg.PadMode.Same
            else:
                attrs.padMode = sg.PadMode.Fixed
                attrs.pads = pads
            attrs.strides = read_strides(op_reader)

            # The kernel shape is inferred at runtime from the input weight tensor.
            op_reader.ignore_attr("kernel_shape")

        case "ConvTranspose":
            attrs = sg.ConvTransposeAttrsT()
            attrs.strides = read_strides(op_reader)

            op_reader.check_attr("auto_pad", "string", "NOTSET")
            op_reader.check_attr("dilations", "ints", ([1], [1, 1]))
            op_reader.check_attr("group", "int", 1)

            # The kernel shape is inferred at runtime from the input weight tensor.
            op_reader.ignore_attr("kernel_shape")

            op_reader.check_attr("output_padding", "ints", [0, 0, 0, 0])
            op_reader.check_attr("pads", "ints", [0, 0, 0, 0])

        case "CumSum":
            op_reader.check_attr("exclusive", "int", 0)
            op_reader.check_attr("reverse", "int", 0)

        case "Flatten":
            attrs = sg.FlattenAttrsT()
            attrs.axis = op_reader.get_attr("axis", "int", 1)

        case "Gather":
            attrs = sg.GatherAttrsT()
            attrs.axis = op_reader.get_attr("axis", "int", 0)

        case "Gemm":
            attrs = sg.GemmAttrsT()
            attrs.alpha = op_reader.get_attr("alpha", "float", 1.0)
            attrs.beta = op_reader.get_attr("beta", "float", 1.0)
            attrs.transposeA = bool(op_reader.get_attr("transA", "int", 0))
            attrs.transposeB = bool(op_reader.get_attr("transB", "int", 0))

        case "GRU":
            attrs = sg.GRUAttrsT()
            attrs.direction = op_reader.get_enum_attr(
                "direction", sg.RNNDirection, "forward"
            )
            attrs.hiddenSize = op_reader.require_attr("hidden_size", "int")
            attrs.linearBeforeReset = bool(
                op_reader.get_attr("linear_before_reset", "int", 0)
            )

        case "HardSigmoid":
            attrs = sg.HardSigmoidAttrsT()
            attrs.alpha = op_reader.get_attr("alpha", "float", 0.2)
            attrs.beta = op_reader.get_attr("beta", "float", 0.5)

        case "LeakyRelu":
            attrs = sg.LeakyReluAttrsT()
            attrs.alpha = op_reader.get_attr("alpha", "float", 0.01)

        case "LogSoftmax":
            attrs = sg.SoftmaxAttrsT()
            attrs.axis = op_reader.get_attr("axis", "int", 0)

        case "LSTM":
            attrs = sg.LSTMAttrsT()
            attrs.direction = op_reader.get_enum_attr(
                "direction", sg.RNNDirection, "forward"
            )
            attrs.hiddenSize = op_reader.require_attr("hidden_size", "int")

            op_reader.check_attr("activation_alpha", "floats", [])
            op_reader.check_attr("activation_beta", "floats", [])
            op_reader.check_attr("activations", "strings", [])
            op_reader.check_attr("clip", "float", 0.0)
            op_reader.check_attr("input_forget", "int", 0)
            op_reader.check_attr("layout", "int", 0)

        case "MaxPool":
            attrs = sg.MaxPoolAttrsT()
            kernel_shape = op_reader.require_attr("kernel_shape", "ints")
            check_ints_length("kernel_shape", kernel_shape, 2)
            attrs.kernelSize = kernel_shape

            pad_mode, pads = read_pads(op_reader)
            if pad_mode == "same":
                attrs.padMode = sg.PadMode.Same
            else:
                attrs.padMode = sg.PadMode.Fixed
                attrs.pads = pads
            attrs.strides = read_strides(op_reader)

            op_reader.check_attr("ceil_mode", "int", 0)
            op_reader.check_attr("dilations", "ints", ([1], [1, 1]))
            op_reader.check_attr("storage_order", "int", 0)

        case "Mod":
            attrs = sg.ModAttrsT()
            attrs.fmod = bool(op_reader.get_attr("fmod", "int", 0))

        case "OneHot":
            attrs = sg.OneHotAttrsT()
            attrs.axis = op_reader.get_attr("axis", "int", -1)

        case "ReduceL2" | "ReduceMax" | "ReduceMean" | "ReduceMin" | "ReduceProd" | "ReduceSum":
            attrs = sg.ReduceMeanAttrsT()
            attrs.axes = op_reader.get_attr("axes", "ints", None)
            attrs.keepDims = bool(op_reader.get_attr("keepdims", "int", 1))

            op_reader.check_attr("noop_with_empty_axes", "int", 0)

        case "Reshape":
            attrs = sg.ReshapeAttrsT()
            attrs.allowZero = bool(op_reader.get_attr("allowzero", "int", 0))

        case "Resize":
            attrs = sg.ResizeAttrsT()
            attrs.mode = op_reader.get_enum_attr("mode", sg.ResizeMode, "nearest")

            op_reader.check_attr("antialias", "int", 0)

            # We only support resizing HW dimensions of NCHW tensor
            op_reader.check_attr("axes", "ints", [2, 3])

            attrs.coordMode = op_reader.get_enum_attr(
                "coordinate_transformation_mode", sg.CoordTransformMode, "half_pixel"
            )

            op_reader.check_attr("cubic_coeff_a", "float", -0.75)
            op_reader.check_attr("exclude_outside", "int", 0)
            op_reader.check_attr("extrapolation_value", "float", 0.0)
            op_reader.check_attr("keep_aspect_ratio_policy", "string", "stretch")

            attrs.nearestMode = op_reader.get_enum_attr(
                "nearest_mode", sg.NearestMode, "round_prefer_floor"
            )

        case "Pad":
            op_reader.check_attr("mode", "string", "constant")

        case "ScatterElements":
            attrs = sg.ScatterElementsAttrsT()
            attrs.axis = op_reader.get_attr("axis", "int", 0)
            attrs.reduction = op_reader.get_enum_attr(
                "reduction", sg.ScatterReduction, "none"
            )

        case "ScatterND":
            attrs = sg.ScatterNDAttrsT()
            attrs.reduction = op_reader.get_enum_attr(
                "reduction", sg.ScatterReduction, "none"
            )

        case "Shape":
            op_reader.check_attr("end", "int", 0)
            op_reader.check_attr("start", "int", 0)

        case "Softmax":
            attrs = sg.SoftmaxAttrsT()
            attrs.axis = op_reader.get_attr("axis", "int", 0)

        case "Split":
            attrs = sg.SplitAttrsT()
            attrs.axis = op_reader.get_attr("axis", "int", 0)
            op_reader.check_attr("num_outputs", "int", 0)
            op_reader.generate_input_from_attr(1, "split", "ints")

        case "Squeeze":
            op_reader.generate_input_from_attr(1, "axes", "ints")

        case "TopK":
            attrs = sg.TopKAttrsT()
            attrs.axis = op_reader.get_attr("axis", "int", -1)
            attrs.largest = bool(op_reader.get_attr("largest", "int", 1))
            attrs.sorted = bool(op_reader.get_attr("sorted", "int", 1))

        case "Transpose":
            attrs = sg.TransposeAttrsT()
            attrs.perm = op_reader.get_attr("perm", "ints", [])

        case "Trilu":
            attrs = sg.TriluAttrsT()
            attrs.upper = bool(op_reader.get_attr("upper", "int", 1))

        case "Unsqueeze":
            op_reader.generate_input_from_attr(1, "axes", "ints")

    if not hasattr(sg.OperatorType, op_type):
        raise Exception(f"Unsupported operator {op_type}")

    # Display a warning for any attributes that were not handled above.
    for attr in op_reader.unhandled_attrs():
        warn_once(
            f"WARNING: Unsupported attribute {attr.name} for operator {onnx_op.op_type}"
        )

    return OperatorNode(
        name=onnx_op.name,
        op_type=op_type,
        attrs=attrs,
        inputs=op_reader.input_indexes,
        outputs=output_indexes,
    )


def graph_from_onnx_graph(onnx_graph: onnx.GraphProto) -> Graph:
    """
    Parse an ONNX model into a graph representation compatible with this library.
    """

    nodes: list[Node] = []

    # Map from tensor ID to node index
    tensor_map: dict[str, int] = {}

    # Map of constant/initializer name to node
    constant_map: dict[str, ConstantNode] = {}

    def add_node(node: Node) -> int:
        if node.name in tensor_map:
            raise Exception(f'Node name "{node.name}" conflicts with another node')
        if isinstance(node, ConstantNode):
            constant_map[node.name] = node
        nodes.append(node)
        node_index = len(nodes) - 1
        tensor_map[node.name] = node_index
        return node_index

    conversion_errors = 0

    for tensor in onnx_graph.initializer:
        try:
            const_node = constant_node_from_onnx_initializer(tensor, None)
            add_node(const_node)
        except Exception as ex:
            warn_once(f"Error converting initializer: {ex}")
            conversion_errors += 1

    for operator in onnx_graph.node:
        if operator.op_type != "Constant":
            continue
        try:
            const_node = constant_node_from_onnx_constant_op(operator)
            add_node(const_node)
        except Exception as ex:
            warn_once(f'Error converting "Constant" operator: {ex}')
            conversion_errors += 1

    # If conversion of any tensors failed, then conversion of any operators
    # which use those tensors will also fail, so we bail early.
    if conversion_errors > 0:
        raise ValueError(
            f"Errors occurred when converting {conversion_errors} constants"
        )

    for value in onnx_graph.input:
        # If the same node is referenced in the ONNX model's `initializer` and
        # `input` properties, ignore the one from the input.
        if value.name in tensor_map:
            continue
        value_node = value_node_from_onnx_value(value)
        add_node(value_node)

    for operator in onnx_graph.node:
        if operator.op_type == "Constant":
            continue

        for output_name in operator.output:
            # TODO - Add shape info for operator outputs, if available.
            value_node = ValueNode(output_name, shape=None)
            add_node(value_node)

        try:
            op_node = op_node_from_onnx_operator(
                operator, tensor_map, constant_map, add_node=add_node
            )
            add_node(op_node)
        except Exception as ex:
            print(
                f"Error converting {operator.op_type} operator {operator.name}: {ex}",
                file=sys.stderr,
            )
            conversion_errors += 1

    if conversion_errors > 0:
        raise ValueError(
            f"Errors occurred when converting {conversion_errors} operators"
        )

    inputs = [tensor_map[info.name] for info in onnx_graph.input]
    outputs = [tensor_map[info.name] for info in onnx_graph.output]
    return Graph(nodes=nodes, inputs=inputs, outputs=outputs)


def build_constant_node(builder: flatbuffers.Builder, constant: ConstantNode):
    """
    Serialize a constant tensor value (eg. model weights) into a FlatBuffers model.
    """
    shape_vec = write_vec(
        builder, sg.ConstantNodeStartShapeVector, constant.shape, "u32"
    )

    # Convert data to NumPy array then serialize. This is much faster than
    # serializing a Python array element by element.
    data_vec = builder.CreateNumpyVector(constant.data.flatten())

    match constant.data.dtype:
        case np.float32:
            sg.FloatDataStart(builder)
            sg.FloatDataAddData(builder, data_vec)
            const_data = sg.FloatDataEnd(builder)
            const_data_type = sg.ConstantData.FloatData
        case np.int32:
            sg.IntDataStart(builder)
            sg.IntDataAddData(builder, data_vec)
            const_data = sg.IntDataEnd(builder)
            const_data_type = sg.ConstantData.IntData
        case _:
            raise ValueError(f"Unsupported data array type {constant.data.dtype.name}")

    sg.ConstantNodeStart(builder)
    sg.ConstantNodeAddShape(builder, shape_vec)
    sg.ConstantNodeAddDataType(builder, const_data_type)
    sg.ConstantNodeAddData(builder, const_data)
    return sg.ConstantNodeEnd(builder)


def write_vec(
    builder: flatbuffers.Builder,
    start_vec,
    data: list[int],
    dtype: Literal["u32", "i32", "offset"],
):
    """
    Serialize a list into a vector in a FlatBuffers buffer.

    `start_vec` is the generated function that starts the vector.
    """
    start_vec(builder, len(data))
    for item in reversed(data):
        match dtype:
            case "u32":
                builder.PrependUint32(item)
            case "i32":
                builder.PrependInt32(item)
            case "offset":
                builder.PrependUOffsetTRelative(item)
            case _:
                raise ValueError("Unsupported data type")
    return builder.EndVector()


def build_operator_node(builder: flatbuffers.Builder, operator: OperatorNode):
    """
    Serialize an operator into a FlatBuffers model.
    """

    if operator.attrs:
        # Given an `operator.attrs` which is an instance of `SomeOpAttrsT`,
        # find the `sg.OperatorAttrs.SomeOpAttrs` constant.
        attrs_const_name = operator.attrs.__class__.__name__[:-1]
        attrs_type = getattr(sg.OperatorAttrs, attrs_const_name)
    else:
        attrs_type = sg.OperatorAttrs.NONE

    operator_table = sg.OperatorNodeT()
    operator_table.type = getattr(sg.OperatorType, operator.op_type)

    operator_table.attrsType = attrs_type
    operator_table.attrs = operator.attrs

    def node_id(maybe_id: int | None) -> int:
        if maybe_id is None:
            return -1
        return maybe_id

    operator_table.inputs = [node_id(id_) for id_ in operator.inputs]
    operator_table.outputs = [node_id(id_) for id_ in operator.outputs]
    return operator_table.Pack(builder)


def build_value_node(builder: flatbuffers.Builder, value: ValueNode):
    """
    Serialize a placeholder for an input/output value into a FlatBuffers model.
    """

    def write_dim(builder, dim: str | int) -> int:
        if isinstance(dim, str):
            name = builder.CreateString(dim)
            sg.DimStart(builder)
            sg.DimAddName(builder, name)
        else:
            sg.DimStart(builder)
            sg.DimAddValue(builder, dim)
        return sg.DimEnd(builder)

    if value.shape is not None:
        dims = [write_dim(builder, dim) for dim in value.shape]
        shape_vec = write_vec(builder, sg.ValueNodeStartShapeVector, dims, "offset")
    else:
        shape_vec = None

    sg.ValueNodeStart(builder)
    if shape_vec:
        sg.ValueNodeAddShape(builder, shape_vec)
    return sg.ValueNodeEnd(builder)


def write_graph(graph: Graph, out_path: str):
    """
    Serialize a model graph into a flatbuffers model.

    This serializes the parsed graph representation into the flatbuffers-based
    model format that this library uses.
    """

    builder = flatbuffers.Builder(initialSize=1024)

    node_offsets = []
    for node in graph.nodes:
        match node:
            case ConstantNode():
                data_type = sg.NodeKind.ConstantNode
                data = build_constant_node(builder, node)
            case OperatorNode():
                data_type = sg.NodeKind.OperatorNode
                data = build_operator_node(builder, node)
            case ValueNode():
                data_type = sg.NodeKind.ValueNode
                data = build_value_node(builder, node)
            case _:
                raise Exception("Unsupported node type")

        name_str = builder.CreateString(node.name)
        sg.NodeStart(builder)
        sg.NodeAddName(builder, name_str)
        sg.NodeAddDataType(builder, data_type)
        sg.NodeAddData(builder, data)
        node_offset = sg.NodeEnd(builder)
        node_offsets.append(node_offset)

    graph_nodes = write_vec(builder, sg.GraphStartNodesVector, node_offsets, "offset")
    inputs = write_vec(builder, sg.GraphStartInputsVector, graph.inputs, "u32")
    outputs = write_vec(builder, sg.GraphStartOutputsVector, graph.outputs, "u32")

    sg.GraphStart(builder)
    sg.GraphAddNodes(builder, graph_nodes)
    sg.GraphAddInputs(builder, inputs)
    sg.GraphAddOutputs(builder, outputs)
    graph = sg.GraphEnd(builder)

    sg.ModelStart(builder)
    sg.ModelAddSchemaVersion(builder, 1)
    sg.ModelAddGraph(builder, graph)
    model = sg.ModelEnd(builder)

    builder.Finish(model)
    data = builder.Output()

    with open(out_path, "wb") as output:
        output.write(data)


def main():
    parser = ArgumentParser()
    parser.add_argument("model", help="Input ONNX model")
    parser.add_argument("out_name", help="Output model file")
    args = parser.parse_args()

    model_path = args.model

    model = onnx.load(model_path)
    graph = graph_from_onnx_graph(model.graph)
    write_graph(graph, args.out_name)


if __name__ == "__main__":
    main()
