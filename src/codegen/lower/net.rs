// C13-2: `lower/stdlib.rs` から net 関連メソッドを機械的に分離 (FB-21 carryover).
//
// This module groups the `Lowering` helpers that describe the
// `taida-lang/net` surface (HTTP v1 / v2 / v3 / v4 / v5, WebSocket,
// SSE) and the enum rewrite that propagates `HttpProtocol` values to
// the native runtime. All items are moved verbatim from
// `src/codegen/lower/stdlib.rs` with their original signatures,
// bodies, and privacy preserved per the C13-2 move-only policy.
//
// Relationship with other submodules:
// - `stdlib.rs` retains stdlib IO / crypto / field-tag registry helpers.
// - `os.rs` holds the `taida-lang/os` package surface + pool helpers.
// - `net.rs` (this file) holds the `taida-lang/net` package surface.

use super::Lowering;
use crate::net_surface::{NET_HTTP_PROTOCOL_VARIANTS, http_protocol_variant_to_wire};
use crate::parser::*;

impl Lowering {
    /// taida-lang/net package function → C runtime function mapping.
    /// Names of taida-lang/net builtins that require scope-aware dispatch.
    /// HTTP v1 (3) + HTTP v2 (1) + HTTP v3 (4) = 8.
    pub(super) const NET_BUILTIN_NAMES: &'static [&'static str] = &[
        "httpServe",
        "httpParseRequestHead",
        "httpEncodeResponse",
        "readBody",
        "startResponse",
        "writeChunk",
        "endResponse",
        "sseEvent",
    ];

    /// Check if a name is a net HTTP v1 builtin that is currently shadowed by a parameter.
    pub(super) fn is_net_builtin_shadowed(&self, name: &str) -> bool {
        self.shadowed_net_builtins.contains(name)
    }

    /// taida-lang/net is HTTP-focused.
    /// Low-level socket/DNS APIs are available from taida-lang/os only.
    pub(super) fn net_func_mapping(sym: &str) -> Option<&'static str> {
        match sym {
            // HTTP v1 surface
            "httpServe" => Some("taida_net_http_serve"),
            "httpParseRequestHead" => Some("taida_net_http_parse_request_head"),
            "httpEncodeResponse" => Some("taida_net_http_encode_response"),
            // HTTP v2 surface
            "readBody" => Some("taida_net_read_body"),
            // HTTP v3 streaming surface
            "startResponse" => Some("taida_net_start_response"),
            "writeChunk" => Some("taida_net_write_chunk"),
            "endResponse" => Some("taida_net_end_response"),
            "sseEvent" => Some("taida_net_sse_event"),
            // HTTP v4 request body streaming surface
            "readBodyChunk" => Some("taida_net_read_body_chunk"),
            "readBodyAll" => Some("taida_net_read_body_all"),
            // HTTP v4 WebSocket surface
            "wsUpgrade" => Some("taida_net_ws_upgrade"),
            "wsSend" => Some("taida_net_ws_send"),
            "wsReceive" => Some("taida_net_ws_receive"),
            "wsClose" => Some("taida_net_ws_close"),
            // v5 WebSocket revision
            "wsCloseCode" => Some("taida_net_ws_close_code"),
            _ => None,
        }
    }

    pub(super) fn register_net_enum_import(&mut self, local_name: &str) {
        self.enum_defs.insert(
            local_name.to_string(),
            NET_HTTP_PROTOCOL_VARIANTS
                .iter()
                .map(|variant| (*variant).to_string())
                .collect(),
        );
    }

    pub(super) fn rewrite_http_serve_tls_expr_for_runtime(&self, expr: &Expr) -> Expr {
        let Expr::BuchiPack(fields, span) = expr else {
            return expr.clone();
        };
        let rewritten_fields = fields
            .iter()
            .map(|field| {
                if field.name == "protocol"
                    && let Expr::EnumVariant(enum_name, variant_name, variant_span) = &field.value
                    && self.enum_defs.contains_key(enum_name)
                {
                    let protocol = http_protocol_variant_to_wire(variant_name);
                    if let Some(protocol) = protocol {
                        let mut rewritten = field.clone();
                        rewritten.value =
                            Expr::StringLit(protocol.to_string(), variant_span.clone());
                        return rewritten;
                    }
                }
                field.clone()
            })
            .collect();
        Expr::BuchiPack(rewritten_fields, span.clone())
    }
}
