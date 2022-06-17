# jq_enum
Build Rust enums from JSON using jq expressions.

You specify expressions that yield an array of strings to be used as variant names,
and optionally additional named expressions returning equally arrays of the same length,
of any serde-deserializable type to associate with the corresponding variant.

The macro then generates an enum with the given name, members,
a getter method for each named associated data element,
and a test which invokes those getter methods to ensure that the jq expression has returned valid deserializable data.

There's an example invocation in [test.rs](tests/test.rs), but realistically the syntax should be substantially reworked.

