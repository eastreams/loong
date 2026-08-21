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
    let error = match ToolPathSegment::try_from("namespace.tool".to_owned()) {
        Ok(_) => panic!("reserved character should be rejected"),
        Err(error) => error,
    };

    assert_eq!(
        error.to_string(),
        "tool path segment contains reserved character '.'"
    );
}
