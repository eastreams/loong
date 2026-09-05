use alloc::{borrow::ToOwned, string::ToString};

use super::{ToolPathSegment, ToolPathSegmentError};

#[test]
fn segment_reports_the_reserved_character() {
    for (value, reserved) in [("namespace.tool", '.'), ("namespace/tool", '/')] {
        let result = ToolPathSegment::try_from(value.to_owned());

        assert!(matches!(
            result,
            Err(ToolPathSegmentError::ReservedCharacter(character))
                if character == reserved
        ));
    }
}

#[test]
fn segment_error_has_actionable_context() {
    let Err(error) = ToolPathSegment::try_from("namespace.tool".to_owned()) else {
        panic!("reserved character should be rejected")
    };

    assert_eq!(
        error.to_string(),
        "tool path segment contains reserved character '.'"
    );
}
