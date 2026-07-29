// The pass fixture covers exports, renaming, and generic bounds.
// Each failure fixture owns one diagnostic.
#[test]
fn message_derive_contract() {
    let tests = trybuild::TestCases::new();
    tests.pass("tests/ui/pass/message.rs");
    tests.compile_fail("tests/ui/fail/*.rs");
}
