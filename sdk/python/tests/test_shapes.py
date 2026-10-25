# SPDX-License-Identifier: Apache-2.0
"""The generated encoders on a fixture proto that has every supported field shape.

`crates/ucdk-codegen/tests/proto/example/shapes/v1/shapes.proto` is compiled by the generator and
its output is kept as a golden next to it (the Rust test `generate.rs` holds the golden to the
generator). This loads that golden as a package of its own and checks the behaviour the Rust
messages have, against bytes written out by hand from the protobuf wire format.
"""

import importlib
import importlib.util
import sys
import unittest
from pathlib import Path

GOLDEN = (
    Path(__file__).resolve().parents[3]
    / "crates/ucdk-codegen/tests/golden/shapes/ucdk/generated"
)


def load():
    spec = importlib.util.spec_from_file_location(
        "shapes_generated", GOLDEN / "__init__.py", submodule_search_locations=[str(GOLDEN)]
    )
    package = importlib.util.module_from_spec(spec)
    sys.modules["shapes_generated"] = package
    spec.loader.exec_module(package)
    return (
        importlib.import_module("shapes_generated.example_shapes_v1"),
        importlib.import_module("shapes_generated._wire"),
    )


shapes, wire = load()
S = shapes.Scalars


def varint(n):
    out = bytearray()
    wire.put_varint(out, n)
    return bytes(out)


def key(number, wire_type):
    return varint((number << 3) | wire_type)


def text(number, s):
    data = s.encode()
    return key(number, 2) + varint(len(data)) + data


def number(field, n):
    return key(field, 0) + varint(n)


NEG1 = b"\xff" * 9 + b"\x01"


class Encoding(unittest.TestCase):
    def test_an_empty_message_is_no_bytes_and_so_is_every_default(self):
        self.assertEqual(S().encode(), b"")
        self.assertEqual(
            S(text="", flag=False, small=0, big=0, usmall=0, ubig=0, mode=shapes.Mode.UNSPECIFIED).encode(),
            b"",
        )
        self.assertEqual(shapes.Empty().encode(), b"")

    def test_integers_booleans_and_strings(self):
        self.assertEqual(S(text="hi").encode(), text(1, "hi"))
        self.assertEqual(S(flag=True).encode(), number(2, 1))
        self.assertEqual(S(small=300).encode(), number(3, 300))
        self.assertEqual(S(usmall=7, ubig=2**63).encode(), number(5, 7) + number(6, 2**63))

    def test_a_negative_integer_is_ten_bytes(self):
        self.assertEqual(S(small=-1).encode(), key(3, 0) + NEG1)
        self.assertEqual(S(big=-1).encode(), key(4, 0) + NEG1)
        self.assertEqual(S(ubig=2**64 - 1).encode(), key(6, 0) + NEG1)

    def test_strings_are_utf8(self):
        self.assertEqual(S(text="é").encode(), b"\x0a\x02\xc3\xa9")

    def test_an_optional_field_that_is_zero_is_written_and_one_that_is_none_is_not(self):
        self.assertEqual(S(opt_small=0).encode(), number(9, 0))
        self.assertEqual(S(opt_text="").encode(), text(8, ""))
        self.assertEqual(S(opt_flag=False).encode(), number(11, 0))
        self.assertEqual(S(opt_mode=shapes.Mode.UNSPECIFIED).encode(), number(10, 0))
        self.assertEqual(S(opt_small=None).encode(), b"")

    def test_an_empty_message_field_is_written_as_a_zero_length_field(self):
        self.assertEqual(S(inner=shapes.Inner()).encode(), key(12, 2) + b"\x00")

    def test_repeated_fields_keep_their_order(self):
        self.assertEqual(S(names=["b", "a"]).encode(), text(13, "b") + text(13, "a"))
        self.assertEqual(
            S(inners=[shapes.Inner(v="x"), shapes.Inner()]).encode(),
            key(14, 2) + b"\x03" + text(1, "x") + key(14, 2) + b"\x00",
        )

    def test_a_map_is_written_in_key_order_and_an_entry_drops_a_default_key_or_value(self):
        self.assertEqual(
            S(labels={"b": "2", "a": "1"}).encode(),
            key(15, 2) + b"\x06" + text(1, "a") + text(2, "1")
            + key(15, 2) + b"\x06" + text(1, "b") + text(2, "2"),
        )
        self.assertEqual(S(counts={"k": 0}).encode(), key(17, 2) + b"\x03" + text(1, "k"))
        self.assertEqual(S(counts={"": 5}).encode(), key(17, 2) + b"\x02" + number(2, 5))
        self.assertEqual(S(inner_map={"": shapes.Inner()}).encode(), key(16, 2) + b"\x00")
        self.assertEqual(
            S(inner_map={"k": shapes.Inner(v="x")}).encode(),
            key(16, 2) + b"\x08" + text(1, "k") + key(2, 2) + b"\x03" + text(1, "x"),
        )
        self.assertEqual(
            S(modes={"m": shapes.Mode.SLOW}).encode(),
            key(18, 2) + b"\x05" + text(1, "m") + number(2, 2),
        )

    def test_a_oneof_arm_is_written_even_when_it_is_the_default(self):
        self.assertEqual(S(a_text="").encode(), text(30, ""))
        self.assertEqual(S(a_int=0).encode(), number(31, 0))
        self.assertEqual(S(a_mode=shapes.Mode.UNSPECIFIED).encode(), number(32, 0))
        self.assertEqual(S(a_empty=shapes.Empty()).encode(), key(35, 2) + b"\x00")

    def test_a_whole_oneof_is_written_where_its_lowest_arm_is(self):
        # `other` has arms 40 and 60 and `nested` is 50: the oneof goes first, as prost writes it.
        value = S(nested=shapes.Scalars_Nested(v="x"), o_flag=True)
        self.assertEqual(
            value.encode(), key(60, 0) + b"\x01" + key(50, 2) + b"\x03" + text(1, "x")
        )
        self.assertEqual(S(o_text="t", nested=shapes.Scalars_Nested()).encode(), text(40, "t") + key(50, 2) + b"\x00")

    def test_more_than_one_arm_set_is_a_mistake(self):
        with self.assertRaises(ValueError):
            S(a_text="x", a_int=1).encode()
        with self.assertRaises(ValueError):
            S(a_text="x", a_int=1).which_choice()

    def test_the_arm_constructors(self):
        self.assertEqual(S.a_text("x").which_choice(), "a_text")
        self.assertEqual(S.a_pair("l", 2, shapes.Mode.FAST), S(a_pair=shapes.Pair("l", 2, shapes.Mode.FAST)))
        self.assertEqual(S.a_inner("v").a_inner, shapes.Inner(v="v"))
        self.assertEqual(S.a_empty().a_empty, shapes.Empty())
        self.assertEqual(S.a_deep("d").a_deep, shapes.Nest_Deep(d="d"))
        self.assertEqual(S.a_nested("n", shapes.Scalars_Nested_Kind.X).a_nested.kind, 1)
        self.assertEqual(S.a_int(5, flag=True), S(a_int=5, flag=True))
        self.assertEqual(S.a_mode(shapes.Mode.SLOW).which_choice(), "a_mode")
        self.assertEqual(S.o_flag(True).which_other(), "o_flag")
        self.assertEqual(S.o_text("t").which_other(), "o_text")
        self.assertIsNone(S().which_choice())

    def test_reserved_words_get_a_trailing_underscore(self):
        value = S(from_="a", encode_="b")
        self.assertEqual(value.encode(), text(19, "a") + text(20, "b"))
        self.assertEqual(S.decode(value.encode()), value)


class Decoding(unittest.TestCase):
    def test_integers_are_read_with_their_sign_and_width(self):
        data = key(3, 0) + NEG1 + key(4, 0) + NEG1 + key(5, 0) + NEG1 + key(6, 0) + NEG1
        value = S.decode(data)
        self.assertEqual((value.small, value.big), (-1, -1))
        self.assertEqual((value.usmall, value.ubig), (2**32 - 1, 2**64 - 1))

    def test_an_int32_is_cut_to_32_bits(self):
        self.assertEqual(S.decode(number(3, 2**32 + 5)).small, 5)
        self.assertEqual(S.decode(number(3, 2**31)).small, -(2**31))

    def test_an_enum_number_this_version_does_not_know_is_kept(self):
        value = S.decode(number(7, 9))
        self.assertEqual(value.mode, 9)
        self.assertNotIsInstance(value.mode, shapes.Mode)
        self.assertEqual(value.encode(), number(7, 9))
        self.assertIsInstance(S.decode(number(7, 2)).mode, shapes.Mode)

    def test_a_negative_enum_number_is_kept(self):
        self.assertEqual(S.decode(key(7, 0) + NEG1).mode, -1)

    def test_optional_fields_that_were_written_are_not_none(self):
        value = S.decode(number(9, 0) + text(8, "") + number(11, 0) + number(10, 0))
        self.assertEqual((value.opt_small, value.opt_text, value.opt_flag), (0, "", False))
        self.assertEqual(value.opt_mode, 0)
        self.assertIsNone(S.decode(b"").opt_small)

    def test_repeated_fields_and_maps(self):
        data = (
            text(13, "a") + text(13, "b")
            + key(15, 2) + b"\x06" + text(1, "k") + text(2, "v")
            + key(17, 2) + b"\x03" + text(1, "z")
            + key(16, 2) + b"\x00"
            + key(18, 2) + b"\x05" + text(1, "m") + number(2, 3)
        )
        value = S.decode(data)
        self.assertEqual(value.names, ["a", "b"])
        self.assertEqual(value.labels, {"k": "v"})
        self.assertEqual(value.counts, {"z": 0})
        self.assertEqual(value.inner_map, {"": shapes.Inner()})
        self.assertEqual(value.modes, {"m": shapes.Mode.MODE_3D})

    def test_an_entry_with_no_key_or_value_reads_them_as_defaults(self):
        value = S.decode(key(15, 2) + b"\x00" + key(15, 2) + b"\x02" + text(2, ""))
        self.assertEqual(value.labels, {"": ""})

    def test_the_last_arm_written_wins(self):
        value = S.decode(text(30, "x") + number(31, 5))
        self.assertEqual((value.a_text, value.a_int), (None, 5))
        value = S.decode(number(31, 5) + key(35, 2) + b"\x00" + text(30, "x"))
        self.assertEqual((value.a_int, value.a_empty, value.a_text), (None, None, "x"))
        self.assertEqual(value.which_choice(), "a_text")

    def test_a_message_field_written_twice_is_merged(self):
        data = key(50, 2) + b"\x05" + text(1, "a") + number(2, 1) + key(50, 2) + b"\x03" + text(1, "b")
        value = S.decode(data)
        self.assertEqual((value.nested.v, value.nested.kind), ("b", 1))

    def test_an_arm_that_is_a_message_written_twice_is_merged(self):
        data = key(37, 2) + b"\x05" + text(1, "a") + number(2, 1) + key(37, 2) + b"\x03" + text(1, "b")
        value = S.decode(data)
        self.assertEqual((value.a_nested.v, value.a_nested.kind), ("b", 1))

    def test_unknown_fields_of_every_wire_type_are_dropped(self):
        data = (
            number(100, 7)
            + key(101, 1) + b"12345678"
            + text(102, "skip me")
            + key(103, 5) + b"1234"
            + text(1, "kept")
        )
        value = S.decode(data)
        self.assertEqual(value.text, "kept")
        self.assertEqual(value.encode(), text(1, "kept"))

    def test_decode_takes_any_bytes_like(self):
        data = text(1, "x")
        for kind in (bytes, bytearray, memoryview):
            self.assertEqual(S.decode(kind(data)).text, "x")

    def test_a_message_decoded_from_nothing_is_the_default_message(self):
        self.assertEqual(S.decode(b""), S())

    def test_nested_enums_and_messages_have_flat_names(self):
        value = S(nested=shapes.Scalars_Nested(v="x", kind=shapes.Scalars_Nested_Kind.X))
        self.assertEqual(S.decode(value.encode()), value)


class Malformed(unittest.TestCase):
    def refuses(self, data):
        with self.assertRaises(wire.DecodeError):
            S.decode(data)

    def test_a_truncated_varint(self):
        self.refuses(b"\x18\x80")
        self.refuses(b"\x80")

    def test_a_varint_longer_than_ten_bytes(self):
        self.refuses(b"\x18" + b"\x80" * 10 + b"\x01")

    def test_a_field_longer_than_the_bytes_left(self):
        self.refuses(b"\x0a\x05ab")
        self.refuses(key(101, 1) + b"123")
        self.refuses(key(103, 5) + b"12")

    def test_field_number_zero(self):
        self.refuses(b"\x00\x01")

    def test_a_wire_type_this_decoder_does_not_read(self):
        self.refuses(key(100, 3))
        self.refuses(key(100, 4))

    def test_a_field_with_the_wrong_wire_type(self):
        self.refuses(number(1, 5))
        self.refuses(text(2, "x"))
        self.refuses(text(7, "x"))
        self.refuses(number(12, 1))
        self.refuses(key(17, 0) + b"\x01")

    def test_a_wrong_wire_type_inside_a_map_entry(self):
        self.refuses(key(15, 2) + b"\x02" + number(1, 1))
        self.refuses(key(17, 2) + b"\x02" + key(2, 2) + b"\x00")

    def test_a_string_that_is_not_utf8(self):
        self.refuses(b"\x0a\x01\xff")

    def test_a_malformed_nested_message(self):
        self.refuses(key(12, 2) + b"\x01\x80")


class Messages(unittest.TestCase):
    def test_equality_compares_type_and_every_field(self):
        self.assertEqual(S(text="a"), S(text="a"))
        self.assertNotEqual(S(text="a"), S(text="b"))
        self.assertNotEqual(S(), shapes.Inner())
        self.assertNotEqual(S(), 5)

    def test_a_message_is_not_hashable(self):
        with self.assertRaises(TypeError):
            hash(S())

    def test_repr_names_the_class_and_the_fields(self):
        self.assertTrue(repr(shapes.Inner(v="x")).startswith("Inner(v='x')"))

    def test_the_base_class_leaves_encoding_to_the_subclass(self):
        with self.assertRaises(NotImplementedError):
            wire.Message().encode()
        with self.assertRaises(NotImplementedError):
            wire.Message.decode(b"")

    def test_set_oneof_picks_the_arm_by_type_and_refuses_another(self):
        target = S()
        wire.set_oneof(target, shapes.Empty(), (("a_inner", shapes.Inner), ("a_empty", shapes.Empty)))
        self.assertEqual(target.a_empty, shapes.Empty())
        wire.set_oneof(target, None, (("a_inner", shapes.Inner),))
        with self.assertRaises(TypeError):
            wire.set_oneof(target, 5, (("a_inner", shapes.Inner),))

    def test_the_modes_have_the_prefix_taken_off_their_names(self):
        self.assertEqual(
            [m.name for m in shapes.Mode], ["UNSPECIFIED", "FAST", "SLOW", "MODE_3D"]
        )
        self.assertEqual(shapes.Scalars_Nested_Kind.X, 1)

    def test_the_docstring_carries_the_proto_comments(self):
        self.assertIn("Every kind of field.", S.__doc__)
        self.assertIn("text: A string.", S.__doc__)
        self.assertIn('\\ and """ in it', S.__doc__)


if __name__ == "__main__":
    unittest.main()
