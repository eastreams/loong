pub(super) fn default_signal_service_url() -> String {
    "http://127.0.0.1:8080".to_owned()
}

pub(super) fn default_signal_account_env() -> Option<String> {
    Some(super::SIGNAL_ACCOUNT_ENV.to_owned())
}

pub(super) fn default_signal_service_url_env() -> Option<String> {
    Some(super::SIGNAL_SERVICE_URL_ENV.to_owned())
}
