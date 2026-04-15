// C12B-024: src/codegen/lower.rs mechanical split (FB-21 / C12-9 Step 2).
//
// Semantics-preserving split of the former monolithic `lower.rs`. This file
// groups stdlib methods of the `Lowering` struct (placement table §2 of
// `.dev/taida-logs/docs/design/file_boundaries.md`). All methods keep their
// original signatures, bodies, and privacy; only the enclosing file changes.

use super::Lowering;
use crate::net_surface::{NET_HTTP_PROTOCOL_VARIANTS, http_protocol_variant_to_wire};
use crate::parser::*;

impl Lowering {
    /// stdout/stderr/stdin → C ランタイム関数名にマッピング (prelude builtins)
    pub(super) fn stdlib_io_mapping(sym: &str) -> Option<&'static str> {
        match sym {
            "stdout" => Some("taida_io_stdout"),
            "stderr" => Some("taida_io_stderr"),
            "stdin" => Some("taida_io_stdin"),
            _ => None,
        }
    }

    /// Register a field type tag, detecting conflicts.
    /// If the same field name is used with different types across different
    /// BuchiPack/Mold definitions (e.g., `Todo.status: Str` vs `HttpResp.status: Int`),
    /// the tag is set to 0 (unknown) so the JSON serializer falls back to
    /// runtime heuristic type detection instead of using the wrong type.
    pub(super) fn register_field_type_tag(&mut self, name: &str, tag: i64) {
        if tag == 0 {
            return;
        }
        if let Some(&existing) = self.field_type_tags.get(name) {
            if existing != tag && existing != 0 {
                // Conflict: same field name used with different types.
                // Set to 0 (unknown) to force runtime heuristic detection.
                self.field_type_tags.insert(name.to_string(), 0);
            }
            // If existing == tag, no change needed (same type, idempotent).
            // If existing == 0, already conflicted, leave it.
        } else {
            self.field_type_tags.insert(name.to_string(), tag);
        }
    }

    /// taida-lang/os package function → C ランタイム関数名にマッピング
    pub(super) fn os_func_mapping(sym: &str) -> Option<&'static str> {
        match sym {
            "readBytes" => Some("taida_os_read_bytes"),
            "writeFile" => Some("taida_os_write_file"),
            "writeBytes" => Some("taida_os_write_bytes"),
            "appendFile" => Some("taida_os_append_file"),
            "remove" => Some("taida_os_remove"),
            "createDir" => Some("taida_os_create_dir"),
            "rename" => Some("taida_os_rename"),
            "run" => Some("taida_os_run"),
            "execShell" => Some("taida_os_exec_shell"),
            "allEnv" => Some("taida_os_all_env"),
            "argv" => Some("taida_os_argv"),
            "dnsResolve" => Some("taida_os_dns_resolve"),
            // Phase 2: async TCP functions
            "tcpConnect" => Some("taida_os_tcp_connect"),
            "tcpListen" => Some("taida_os_tcp_listen"),
            "tcpAccept" => Some("taida_os_tcp_accept"),
            "socketSend" => Some("taida_os_socket_send"),
            "socketSendAll" => Some("taida_os_socket_send_all"),
            "socketRecv" => Some("taida_os_socket_recv"),
            "socketSendBytes" => Some("taida_os_socket_send_bytes"),
            "socketRecvBytes" => Some("taida_os_socket_recv_bytes"),
            "socketRecvExact" => Some("taida_os_socket_recv_exact"),
            "udpBind" => Some("taida_os_udp_bind"),
            "udpSendTo" => Some("taida_os_udp_send_to"),
            "udpRecvFrom" => Some("taida_os_udp_recv_from"),
            "socketClose" => Some("taida_os_socket_close"),
            "listenerClose" => Some("taida_os_listener_close"),
            "udpClose" => Some("taida_os_socket_close"),
            // Mold names (Read, ListDir, Stat, Exists, EnvVar, ReadAsync, Http*) are handled in lower_molds.rs
            "Read" | "ListDir" | "Stat" | "Exists" | "EnvVar" | "ReadAsync" | "HttpGet"
            | "HttpPost" | "HttpRequest" => Some("__os_mold__"),
            _ => None,
        }
    }

    /// taida-lang/crypto package function → C runtime function mapping.
    pub(super) fn crypto_func_mapping(sym: &str) -> Option<&'static str> {
        match sym {
            "sha256" => Some("taida_sha256"),
            _ => None,
        }
    }

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

    /// taida-lang/pool package function → C runtime function mapping.
    pub(super) fn pool_func_mapping(sym: &str) -> Option<&'static str> {
        match sym {
            "poolCreate" => Some("taida_pool_create"),
            "poolAcquire" => Some("taida_pool_acquire"),
            "poolRelease" => Some("taida_pool_release"),
            "poolClose" => Some("taida_pool_close"),
            "poolHealth" => Some("taida_pool_health"),
            _ => None,
        }
    }
}
