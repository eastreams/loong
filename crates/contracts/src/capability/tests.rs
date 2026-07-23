use super::{Capabilities, Capability};

const READ_CAPABILITIES: Capabilities = Capabilities::singleton(Capability::FsRead);
const READ_WRITE_CAPABILITIES: Capabilities = READ_CAPABILITIES.with(Capability::FsWrite);

#[test]
fn const_construction_matches_runtime_construction() {
    assert_eq!(READ_CAPABILITIES, Capability::FsRead.into());
    assert_eq!(
        READ_WRITE_CAPABILITIES,
        [Capability::FsRead, Capability::FsWrite]
            .into_iter()
            .collect()
    );
}

#[test]
fn empty_capabilities_iterator_stays_exhausted() {
    let mut iter = Capabilities::default().into_iter();

    assert_eq!(iter.next(), None);
    assert_eq!(iter.next(), None);
}

#[test]
fn empty_capabilities_have_stable_wire_format() -> serde_json::Result<()> {
    let encoded = serde_json::to_string(&Capabilities::default())?;
    assert_eq!(encoded, "[]");

    let decoded = serde_json::from_str::<Capabilities>(&encoded)?;
    assert_eq!(decoded, Capabilities::default());
    Ok(())
}

#[test]
fn capabilities_have_canonical_wire_format() -> serde_json::Result<()> {
    let decoded = serde_json::from_str::<Capabilities>(r#"["fs.write", "fs.read", "fs.write"]"#)?;

    assert_eq!(
        serde_json::to_string(&decoded)?,
        r#"["fs.read","fs.write"]"#
    );
    assert_eq!(serde_json::to_string(&Capability::FsRead)?, r#""fs.read""#);
    assert_eq!(
        serde_json::from_str::<Capability>(r#""fs.write""#)?,
        Capability::FsWrite
    );
    Ok(())
}

#[test]
fn binary_wire_format_uses_stable_names() -> Result<(), bincode::Error> {
    let capability = bincode::serialize(&Capability::FsRead)?;
    assert_eq!(capability, bincode::serialize("fs.read")?);
    assert_eq!(
        bincode::deserialize::<Capability>(&capability)?,
        Capability::FsRead
    );

    let capabilities = READ_WRITE_CAPABILITIES;
    let encoded = bincode::serialize(&capabilities)?;
    assert_eq!(encoded, bincode::serialize(&vec!["fs.read", "fs.write"])?);
    assert_eq!(
        bincode::deserialize::<Capabilities>(&encoded)?,
        capabilities
    );
    Ok(())
}

#[test]
fn capabilities_reject_values_outside_the_wire_contract() {
    assert!(serde_json::from_str::<Capabilities>(r#"["unknown"]"#).is_err());
    assert!(serde_json::from_str::<Capabilities>("[0]").is_err());
    assert!(serde_json::from_str::<Capabilities>("{}").is_err());
}

#[test]
fn capabilities_schema_describes_the_wire_set() -> serde_json::Result<()> {
    let schema = serde_json::to_value(schemars::schema_for!(Capabilities))?;

    assert_eq!(
        schema.get("type").and_then(serde_json::Value::as_str),
        Some("array")
    );
    assert_eq!(
        schema
            .get("uniqueItems")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        schema
            .pointer("/items/$ref")
            .and_then(serde_json::Value::as_str),
        Some("#/$defs/Capability")
    );
    assert_eq!(
        schema.pointer("/$defs/Capability/enum"),
        Some(&serde_json::json!(["fs.read", "fs.write"]))
    );
    Ok(())
}
