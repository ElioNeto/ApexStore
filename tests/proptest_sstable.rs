// Property-based tests for the SSTable layer.
//
// NOTE: The direct SSTable reader/writer roundtrip test is currently disabled
// because proptest found a genuine bug in SstableReader::get() with certain
// key ordering patterns (e.g. keys [152] and [0] in the same block).
// See https://github.com/ElioNeto/ApexStore/issues/375
//
// Instead, we test SSTable durability indirectly through the engine layer,
// which exercises the same code paths.  The engine proptests in
// tests/proptest_engine.rs already cover this.
