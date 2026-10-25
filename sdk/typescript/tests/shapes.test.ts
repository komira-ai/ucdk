// SPDX-License-Identifier: Apache-2.0
/**
 * The generated encoders on a fixture proto that has every supported field shape.
 *
 * `crates/ucdk-codegen/tests/proto/example/shapes/v1/shapes.proto` is compiled by the generator
 * and its output is kept as a golden next to it (the Rust test `generate.rs` holds the golden to
 * the generator). This imports that golden as a package of its own and checks the behaviour the
 * Rust messages have, against bytes written out by hand from the protobuf wire format.
 */

import assert from "node:assert/strict";
import { test } from "node:test";

import * as shapes from "../../../crates/ucdk-codegen/tests/golden/shapes/src/generated/example_shapes_v1.ts";
import * as wire from "../../../crates/ucdk-codegen/tests/golden/shapes/src/generated/wire.ts";

const S = shapes.Scalars;

function varint(n: number | bigint): Buffer {
  const out = new wire.Out();
  wire.putVarint(out, n);
  return Buffer.from(out.bytes());
}

function key(number: number, wireType: number): Buffer {
  return varint(number * 8 + wireType);
}

function text(number: number, s: string): Buffer {
  const data = Buffer.from(s);
  return Buffer.concat([key(number, 2), varint(data.length), data]);
}

function num(field: number, n: number | bigint): Buffer {
  return Buffer.concat([key(field, 0), varint(n)]);
}

function cat(...parts: Buffer[]): Buffer {
  return Buffer.concat(parts);
}

function enc(message: { encode(): Uint8Array }): Buffer {
  return Buffer.from(message.encode());
}

const NEG1 = Buffer.from([0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01]);
const bytes = (...values: number[]): Buffer => Buffer.from(values);

// ---- encoding ------------------------------------------------------------------------------

test("an empty message is no bytes and so is every default", () => {
  assert.deepEqual(enc(new S()), Buffer.alloc(0));
  assert.deepEqual(
    enc(
      new S({
        text: "",
        flag: false,
        small: 0,
        big: 0n,
        usmall: 0,
        ubig: 0n,
        mode: shapes.Mode.UNSPECIFIED,
      }),
    ),
    Buffer.alloc(0),
  );
  assert.deepEqual(enc(new shapes.Empty()), Buffer.alloc(0));
});

test("integers, booleans and strings", () => {
  assert.deepEqual(enc(new S({ text: "hi" })), text(1, "hi"));
  assert.deepEqual(enc(new S({ flag: true })), num(2, 1));
  assert.deepEqual(enc(new S({ small: 300 })), num(3, 300));
  assert.deepEqual(
    enc(new S({ usmall: 7, ubig: 2n ** 63n })),
    cat(num(5, 7), num(6, 2n ** 63n)),
  );
});

test("a negative integer is ten bytes", () => {
  assert.deepEqual(enc(new S({ small: -1 })), cat(key(3, 0), NEG1));
  assert.deepEqual(enc(new S({ big: -1n })), cat(key(4, 0), NEG1));
  assert.deepEqual(enc(new S({ ubig: 2n ** 64n - 1n })), cat(key(6, 0), NEG1));
});

test("strings are utf8", () => {
  assert.deepEqual(enc(new S({ text: "é" })), bytes(0x0a, 0x02, 0xc3, 0xa9));
});

test("a string keeps a byte order mark and an unpaired surrogate becomes the replacement character", () => {
  assert.equal(S.decode(enc(new S({ text: "\uFEFFx" }))).text, "\uFEFFx");
  assert.equal(S.decode(enc(new S({ text: "a\uD800" }))).text, "a\uFFFD");
});

test("an optional field that is zero is written and one that is undefined is not", () => {
  assert.deepEqual(enc(new S({ optSmall: 0 })), num(9, 0));
  assert.deepEqual(enc(new S({ optText: "" })), text(8, ""));
  assert.deepEqual(enc(new S({ optFlag: false })), num(11, 0));
  assert.deepEqual(enc(new S({ optMode: shapes.Mode.UNSPECIFIED })), num(10, 0));
  assert.deepEqual(enc(new S({ optSmall: undefined })), Buffer.alloc(0));
});

test("an empty message field is written as a zero length field", () => {
  assert.deepEqual(enc(new S({ inner: new shapes.Inner() })), cat(key(12, 2), bytes(0)));
});

test("repeated fields keep their order", () => {
  assert.deepEqual(enc(new S({ names: ["b", "a"] })), cat(text(13, "b"), text(13, "a")));
  assert.deepEqual(
    enc(new S({ inners: [new shapes.Inner({ v: "x" }), new shapes.Inner()] })),
    cat(key(14, 2), bytes(3), text(1, "x"), key(14, 2), bytes(0)),
  );
});

test("a map is written in key order and an entry drops a default key or value", () => {
  assert.deepEqual(
    enc(new S({ labels: { b: "2", a: "1" } })),
    cat(
      key(15, 2),
      bytes(6),
      text(1, "a"),
      text(2, "1"),
      key(15, 2),
      bytes(6),
      text(1, "b"),
      text(2, "2"),
    ),
  );
  assert.deepEqual(enc(new S({ counts: { k: 0 } })), cat(key(17, 2), bytes(3), text(1, "k")));
  assert.deepEqual(enc(new S({ counts: { "": 5 } })), cat(key(17, 2), bytes(2), num(2, 5)));
  assert.deepEqual(
    enc(new S({ innerMap: { "": new shapes.Inner() } })),
    cat(key(16, 2), bytes(0)),
  );
  assert.deepEqual(
    enc(new S({ innerMap: { k: new shapes.Inner({ v: "x" }) } })),
    cat(key(16, 2), bytes(8), text(1, "k"), key(2, 2), bytes(3), text(1, "x")),
  );
  assert.deepEqual(
    enc(new S({ modes: { m: shapes.Mode.SLOW } })),
    cat(key(18, 2), bytes(5), text(1, "m"), num(2, 2)),
  );
});

test("a map is taken from a Map or from an object, and is a copy", () => {
  const given = new Map([["a", "1"]]);
  const fromMap = new S({ labels: given });
  given.set("b", "2");
  assert.deepEqual([...fromMap.labels], [["a", "1"]]);
  assert.deepEqual(enc(fromMap), enc(new S({ labels: { a: "1" } })));
});

test("map keys are put in the order of their utf8 bytes, not of their utf16 units", () => {
  // U+1F600 is the UTF-16 pair D83D DE00, which JavaScript sorts before U+FF5E; its UTF-8 bytes
  // (F0 9F 98 80) sort after those of U+FF5E (EF BD 9E), which is the order a map is written in.
  const keys = ["\u{1F600}", "～"];
  const value = new S({ labels: Object.fromEntries(keys.map((k) => [k, "v"])) });
  assert.deepEqual(
    [...S.decode(enc(value)).labels.keys()],
    ["～", "\u{1F600}"],
  );
  assert.ok("\u{1F600}" < "～", "JavaScript's own order is the other one");
});

test("a oneof arm is written even when it is the default", () => {
  assert.deepEqual(enc(new S({ aText: "" })), text(30, ""));
  assert.deepEqual(enc(new S({ aInt: 0n })), num(31, 0));
  assert.deepEqual(enc(new S({ aMode: shapes.Mode.UNSPECIFIED })), num(32, 0));
  assert.deepEqual(enc(new S({ aEmpty: new shapes.Empty() })), cat(key(35, 2), bytes(0)));
});

test("a whole oneof is written where its lowest arm is", () => {
  // `other` has arms 40 and 60 and `nested` is 50: the oneof goes first, as prost writes it.
  const value = new S({ nested: new shapes.Scalars_Nested({ v: "x" }), oFlag: true });
  assert.deepEqual(enc(value), cat(key(60, 0), bytes(1), key(50, 2), bytes(3), text(1, "x")));
  assert.deepEqual(
    enc(new S({ oText: "t", nested: new shapes.Scalars_Nested() })),
    cat(text(40, "t"), key(50, 2), bytes(0)),
  );
});

test("more than one arm set is a mistake", () => {
  const both = new S({ aText: "x", aInt: 1n });
  assert.throws(() => both.encode(), /more than one of aText, aInt is set/);
  assert.throws(() => both.whichChoice(), Error);
});

test("the arm factories", () => {
  assert.equal(S.aText("x").whichChoice(), "aText");
  assert.deepEqual(
    S.aPair("l", 2, shapes.Mode.FAST),
    new S({ aPair: new shapes.Pair({ left: "l", right: 2, speed: shapes.Mode.FAST }) }),
  );
  assert.deepEqual(S.aInner("v").aInner, new shapes.Inner({ v: "v" }));
  assert.deepEqual(S.aEmpty().aEmpty, new shapes.Empty());
  assert.deepEqual(S.aDeep("d").aDeep, new shapes.Nest_Deep({ d: "d" }));
  assert.equal(S.aNested("n", shapes.Scalars_Nested_Kind.X).aNested?.kind, 1);
  assert.deepEqual(S.aInt(5n, { flag: true }), new S({ aInt: 5n, flag: true }));
  assert.equal(S.aMode(shapes.Mode.SLOW).whichChoice(), "aMode");
  assert.equal(S.oFlag(true).whichOther(), "oFlag");
  assert.equal(S.oText("t").whichOther(), "oText");
  assert.equal(new S().whichChoice(), undefined);
});

test("a parameter that is a word keeps the property's name", () => {
  const words = S.aWords("d", "o").aWords;
  assert.deepEqual(words, new shapes.Words({ delete: "d", options: "o" }));
  assert.deepEqual(enc(new shapes.Words({ delete: "d", options: "o" })), cat(text(1, "d"), text(2, "o")));
});

test("a name a message keeps gets a trailing underscore", () => {
  const value = new S({ from: "a", encode_: "b" });
  assert.deepEqual(enc(value), cat(text(19, "a"), text(20, "b")));
  assert.deepEqual(S.decode(enc(value)), value);
});

// ---- decoding ------------------------------------------------------------------------------

test("integers are read with their sign and width", () => {
  const value = S.decode(cat(key(3, 0), NEG1, key(4, 0), NEG1, key(5, 0), NEG1, key(6, 0), NEG1));
  assert.deepEqual([value.small, value.big], [-1, -1n]);
  assert.deepEqual([value.usmall, value.ubig], [2 ** 32 - 1, 2n ** 64n - 1n]);
});

test("an int32 is cut to 32 bits", () => {
  assert.equal(S.decode(num(3, 2n ** 32n + 5n)).small, 5);
  assert.equal(S.decode(num(3, 2n ** 31n)).small, -(2 ** 31));
});

test("an enum number this version does not know is kept", () => {
  const value = S.decode(num(7, 9));
  assert.equal(value.mode, 9);
  assert.deepEqual(enc(value), num(7, 9));
  assert.equal(S.decode(num(7, 2)).mode, shapes.Mode.SLOW);
});

test("a negative enum number is kept", () => {
  assert.equal(S.decode(cat(key(7, 0), NEG1)).mode, -1);
});

test("optional fields that were written are not undefined", () => {
  const value = S.decode(cat(num(9, 0), text(8, ""), num(11, 0), num(10, 0)));
  assert.deepEqual([value.optSmall, value.optText, value.optFlag], [0, "", false]);
  assert.equal(value.optMode, 0);
  assert.equal(S.decode(Buffer.alloc(0)).optSmall, undefined);
});

test("repeated fields and maps", () => {
  const value = S.decode(
    cat(
      text(13, "a"),
      text(13, "b"),
      key(15, 2),
      bytes(6),
      text(1, "k"),
      text(2, "v"),
      key(17, 2),
      bytes(3),
      text(1, "z"),
      key(16, 2),
      bytes(0),
      key(18, 2),
      bytes(5),
      text(1, "m"),
      num(2, 3),
    ),
  );
  assert.deepEqual(value.names, ["a", "b"]);
  assert.deepEqual(value.labels, new Map([["k", "v"]]));
  assert.deepEqual(value.counts, new Map([["z", 0]]));
  assert.deepEqual(value.innerMap, new Map([["", new shapes.Inner()]]));
  assert.deepEqual(value.modes, new Map([["m", shapes.Mode.MODE_3D]]));
});

test("an entry with no key or value reads them as defaults", () => {
  const value = S.decode(cat(key(15, 2), bytes(0), key(15, 2), bytes(2), text(2, "")));
  assert.deepEqual(value.labels, new Map([["", ""]]));
});

test("the last arm written wins", () => {
  let value = S.decode(cat(text(30, "x"), num(31, 5)));
  assert.deepEqual([value.aText, value.aInt], [undefined, 5n]);
  value = S.decode(cat(num(31, 5), key(35, 2), bytes(0), text(30, "x")));
  assert.deepEqual([value.aInt, value.aEmpty, value.aText], [undefined, undefined, "x"]);
  assert.equal(value.whichChoice(), "aText");
});

test("a message field written twice is merged", () => {
  const data = cat(key(50, 2), bytes(5), text(1, "a"), num(2, 1), key(50, 2), bytes(3), text(1, "b"));
  const value = S.decode(data);
  assert.deepEqual([value.nested?.v, value.nested?.kind], ["b", 1]);
});

test("an arm that is a message written twice is merged", () => {
  const data = cat(key(37, 2), bytes(5), text(1, "a"), num(2, 1), key(37, 2), bytes(3), text(1, "b"));
  const value = S.decode(data);
  assert.deepEqual([value.aNested?.v, value.aNested?.kind], ["b", 1]);
});

test("unknown fields of every wire type are dropped", () => {
  const data = cat(
    num(100, 7),
    key(101, 1),
    Buffer.from("12345678"),
    text(102, "skip me"),
    key(103, 5),
    Buffer.from("1234"),
    text(1, "kept"),
  );
  const value = S.decode(data);
  assert.equal(value.text, "kept");
  assert.deepEqual(enc(value), text(1, "kept"));
});

test("decode takes any array of bytes", () => {
  const data = text(1, "x");
  for (const given of [new Uint8Array(data), Buffer.from(data), data.subarray(0)]) {
    assert.equal(S.decode(given).text, "x");
  }
});

test("a message decoded from nothing is the default message", () => {
  assert.deepEqual(S.decode(new Uint8Array(0)), new S());
});

test("nested enums and messages have flat names", () => {
  const value = new S({
    nested: new shapes.Scalars_Nested({ v: "x", kind: shapes.Scalars_Nested_Kind.X }),
  });
  assert.deepEqual(S.decode(enc(value)), value);
});

test("a message with no fields still checks the bytes it is given", () => {
  assert.deepEqual(shapes.Empty.decode(num(9, 1)), new shapes.Empty());
  assert.throws(() => shapes.Empty.decode(bytes(0x80)), wire.DecodeError);
});

// ---- malformed bytes -----------------------------------------------------------------------

function refuses(data: Uint8Array): void {
  assert.throws(() => S.decode(data), wire.DecodeError);
}

test("a truncated varint", () => {
  refuses(bytes(0x18, 0x80));
  refuses(bytes(0x80));
});

test("a varint longer than ten bytes", () => {
  refuses(cat(bytes(0x18), Buffer.alloc(10, 0x80), bytes(0x01)));
});

test("a field longer than the bytes left", () => {
  refuses(bytes(0x0a, 0x05, 0x61, 0x62));
  refuses(cat(key(101, 1), Buffer.from("123")));
  refuses(cat(key(103, 5), Buffer.from("12")));
});

test("field number zero", () => {
  refuses(bytes(0x00, 0x01));
});

test("a wire type this decoder does not read", () => {
  refuses(key(100, 3));
  refuses(key(100, 4));
});

test("a field with the wrong wire type", () => {
  refuses(num(1, 5));
  refuses(text(2, "x"));
  refuses(text(7, "x"));
  refuses(num(12, 1));
  refuses(cat(key(17, 0), bytes(1)));
});

test("a wrong wire type inside a map entry", () => {
  refuses(cat(key(15, 2), bytes(2), num(1, 1)));
  refuses(cat(key(17, 2), bytes(2), key(2, 2), bytes(0)));
});

test("a string that is not utf8", () => {
  refuses(bytes(0x0a, 0x01, 0xff));
});

test("a malformed nested message", () => {
  refuses(cat(key(12, 2), bytes(0x01, 0x80)));
});

// ---- messages ------------------------------------------------------------------------------

test("a message holds what it was given and copies what it iterates", () => {
  const names = ["a"];
  const value = new S({ names });
  names.push("b");
  assert.deepEqual(value.names, ["a"]);
  assert.deepEqual(new S({ names: new Set(["x", "y"]) }).names, ["x", "y"]);
});

test("equality compares type and every field", () => {
  assert.deepEqual(new S({ text: "a" }), new S({ text: "a" }));
  assert.notDeepEqual(new S({ text: "a" }), new S({ text: "b" }));
  assert.notDeepEqual(new S(), new shapes.Inner());
});

test("the modes are constants with the prefix taken off their names", () => {
  assert.deepEqual(Object.keys(shapes.Mode), ["UNSPECIFIED", "FAST", "SLOW", "MODE_3D"]);
  assert.equal(shapes.Scalars_Nested_Kind.X, 1);
});
