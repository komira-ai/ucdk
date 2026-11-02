# Declared names on a Publisher

A release declares the names of the artifacts it publishes, and a `Publisher` is told them with
`Publisher::declare` before the release stages anything. The rule is the one kci's publish path
applies (its approved-names list): a name that was not declared is never uploaded, even by a bug
above the publisher.

## What a publisher does

- `stage` and `commit` refuse a call that names a coordinate whose name is not declared with
  `SeamError::Undeclared { name }`, which names the artifact, promises that nothing happened, and
  is checked before anything is sent. A call that holds one undeclared member is refused whole.
- A publisher fails closed. One that was never told any names declares none and refuses every stage
  and commit; so does one told an empty list.
- `declare` replaces the declaration, and the new one governs a commit of state staged under the
  old one: a commit of a name no longer declared is refused and the staged state stays until it is
  aborted. `abort` is not refused for a name.
- A declaration is checked whole by `ucdk_release_seams::check_declaration`. It refuses an empty
  name, a name that holds whitespace or a `/`, and a name that is an earlier one again ignoring
  ASCII case, with a `DeclareError` that names the entry; a refused declaration changes nothing.
- A failed preflight observe that carries `Undeclared` is refused with `APPROVED_NAMES`.

## What a name is compared as

Names are compared as declared, ignoring ASCII case and nothing else.

- Non-ASCII letters are not case-folded: `\u{c9}` and `\u{e9}` are two names.
- Whitespace is not trimmed; a name that holds any is refused when declared.
- kci's PEP 503 normalisation, which applies to a Python index only, is not applied. A release that
  declares `my_pkg` and stages `my-pkg` is refused on such a registry, though the registry
  would treat them as one project. That is a false refusal, never a false admission: two names
  compared equal here are equal to every registry.

## How the kit holds it

The publisher kit binds every implementation to this: a publisher refuses an undeclared artifact
(including a set with one, and a commit after a re-declaration), a publisher never told any names
refuses, and a declaration that is not plain distinct names is refused and changes nothing. The
kit's own scenarios declare the names they publish before they publish.
