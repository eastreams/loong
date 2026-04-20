use crate::TransportInfo;

pub fn test_transport_info(name: &str) -> TransportInfo {
    TransportInfo {
        name: name.to_owned(),
        version: "0.1.0-test".to_owned(),
        secure: false,
    }
}
