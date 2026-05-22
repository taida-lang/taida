use super::Lowering;

impl Lowering {
    /// taida-lang/abi package function -> C runtime function mapping.
    pub(super) fn abi_func_mapping(sym: &str) -> Option<&'static str> {
        match sym {
            "text" => Some("taida_abi_response_text"),
            "json" => Some("taida_abi_response_json"),
            "bytes" => Some("taida_abi_response_bytes"),
            "status" => Some("taida_abi_response_status"),
            "header" => Some("taida_abi_response_header"),
            _ => None,
        }
    }
}
