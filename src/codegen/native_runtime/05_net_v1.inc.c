// ── taida-lang/net: HTTP v1 runtime ─────────────────────────────
// httpParseRequestHead, httpEncodeResponse, httpServe
// These are dedicated net runtime functions, not os wrappers.

// Forward declarations
taida_val taida_net_http_parse_request_head(taida_val input);
taida_val taida_net_http_encode_response(taida_val response);
taida_val taida_net_http_serve(taida_val port, taida_val handler, taida_val max_requests, taida_val timeout_ms, taida_val max_connections, taida_val tls, taida_val handler_type_tag, taida_val handler_arity);
taida_val taida_net_read_body(taida_val req);
// NET3-5b: v3 streaming API forward declarations
taida_val taida_net_start_response(taida_val writer, taida_val status, taida_val headers);
taida_val taida_net_write_chunk(taida_val writer, taida_val data);
taida_val taida_net_end_response(taida_val writer);
taida_val taida_net_sse_event(taida_val writer, taida_val event, taida_val data);
// NB4-6: v4 request body streaming + WebSocket API forward declarations
taida_val taida_net_read_body_chunk(taida_val req);
taida_val taida_net_read_body_all(taida_val req);
taida_val taida_net_ws_upgrade(taida_val req, taida_val writer);
taida_val taida_net_ws_send(taida_val ws, taida_val data);
taida_val taida_net_ws_receive(taida_val ws);
taida_val taida_net_ws_close(taida_val ws, taida_val code);
taida_val taida_net_ws_close_code(taida_val ws);
// v4: body stream request check (defined later, forward declared here for readBody delegation)
static int taida_net4_is_body_stream_request(taida_val req);

// Net result helpers (use HttpError instead of IoError)
static taida_val taida_net_result_ok(taida_val inner) {
    return taida_result_create(inner, 0, 0);
}

static taida_val taida_net_result_fail(const char *kind, const char *message) {
    // inner = @(ok: false, code: -1, message: msg, kind: kind)
    taida_val inner = taida_pack_new(4);
    taida_pack_set_hash(inner, 0, taida_str_hash((taida_val)"ok"));
    taida_pack_set(inner, 0, 0);  // false
    taida_pack_set_tag(inner, 0, TAIDA_TAG_BOOL);
    taida_pack_set_hash(inner, 1, taida_str_hash((taida_val)"code"));
    taida_pack_set(inner, 1, -1);
    taida_pack_set_hash(inner, 2, taida_str_hash((taida_val)"message"));
    taida_pack_set(inner, 2, (taida_val)taida_str_new_copy(message));
    taida_pack_set_tag(inner, 2, TAIDA_TAG_STR);
    taida_pack_set_hash(inner, 3, taida_str_hash((taida_val)"kind"));
    taida_pack_set(inner, 3, (taida_val)taida_str_new_copy(kind));
    taida_pack_set_tag(inner, 3, TAIDA_TAG_STR);

    taida_val error = taida_make_error("HttpError", message);
    return taida_result_create(inner, error, 0);
}

// Helper: create span @(start: Int, len: Int)
static taida_val taida_net_make_span(taida_val start, taida_val len) {
    taida_val pack = taida_pack_new(2);
    taida_pack_set_hash(pack, 0, taida_str_hash((taida_val)"start"));
    taida_pack_set(pack, 0, start);
    taida_pack_set_hash(pack, 1, taida_str_hash((taida_val)"len"));
    taida_pack_set(pack, 1, len);
    return pack;
}

// Status reason phrases (mirrors Interpreter status_reason)
static const char *taida_net_status_reason(int code) {
    switch (code) {
        case 100: return "Continue";
        case 101: return "Switching Protocols";
        case 200: return "OK";
        case 201: return "Created";
        case 202: return "Accepted";
        case 204: return "No Content";
        case 205: return "Reset Content";
        case 206: return "Partial Content";
        case 301: return "Moved Permanently";
        case 302: return "Found";
        case 304: return "Not Modified";
        case 307: return "Temporary Redirect";
        case 308: return "Permanent Redirect";
        case 400: return "Bad Request";
        case 401: return "Unauthorized";
        case 403: return "Forbidden";
        case 404: return "Not Found";
        case 405: return "Method Not Allowed";
        case 408: return "Request Timeout";
        case 409: return "Conflict";
        case 410: return "Gone";
        case 413: return "Content Too Large";
        case 415: return "Unsupported Media Type";
        case 418: return "I'm a Teapot";
        case 422: return "Unprocessable Content";
        case 429: return "Too Many Requests";
        case 500: return "Internal Server Error";
        case 502: return "Bad Gateway";
        case 503: return "Service Unavailable";
        case 504: return "Gateway Timeout";
        default:  return "";
    }
}

// ── httpParseRequestHead(bytes) ─────────────────────────────────
// Hand-written HTTP/1.1 request head parser (no external deps).
// Returns Result[@(complete, consumed, method, path, query, version, headers, bodyOffset, contentLength, chunked), _]
taida_val taida_net_http_parse_request_head(taida_val input) {
    // Extract raw bytes from Bytes or Str
    unsigned char *data = NULL;
    size_t data_len = 0;
    int free_data = 0;

    if (TAIDA_IS_BYTES(input)) {
        taida_val *bytes = (taida_val*)input;
        taida_val blen = bytes[1];
        if (blen < 0) blen = 0;
        data_len = (size_t)blen;
        data = (unsigned char*)TAIDA_MALLOC(data_len + 1, "net_parse_input");
        for (size_t i = 0; i < data_len; i++) data[i] = (unsigned char)bytes[2 + i];
        data[data_len] = 0;
        free_data = 1;
    } else {
        // Assume string
        size_t slen = 0;
        if (!taida_read_cstr_len_safe((const char*)input, 1048576, &slen)) {
            return taida_net_result_fail("ParseError", "httpParseRequestHead: argument must be Bytes or Str");
        }
        data = (unsigned char*)input;
        data_len = slen;
    }

    // Find \r\n\r\n (end of head)
    int head_end = -1;
    for (size_t i = 0; i + 3 < data_len; i++) {
        if (data[i] == '\r' && data[i+1] == '\n' && data[i+2] == '\r' && data[i+3] == '\n') {
            head_end = (int)i;
            break;
        }
    }

    int complete = (head_end >= 0);
    size_t consumed = complete ? (size_t)(head_end + 4) : 0;

    // We need at least a request line to parse
    // Find the first \r\n for request line
    int first_crlf = -1;
    size_t scan_limit = complete ? (size_t)head_end : data_len;
    for (size_t i = 0; i + 1 < scan_limit; i++) {
        if (data[i] == '\r' && data[i+1] == '\n') {
            first_crlf = (int)i;
            break;
        }
    }

    if (first_crlf < 0) {
        // No CRLF found at all — incomplete if no head_end, try to check for obvious malformed
        if (!complete) {
            // Could be incomplete — return incomplete result
            taida_val parsed = taida_pack_new(10);
            taida_pack_set_hash(parsed, 0, taida_str_hash((taida_val)"complete"));
            taida_pack_set(parsed, 0, 0);  // false
            taida_pack_set_tag(parsed, 0, TAIDA_TAG_BOOL);
            taida_pack_set_hash(parsed, 1, taida_str_hash((taida_val)"consumed"));
            taida_pack_set(parsed, 1, 0);
            taida_pack_set_hash(parsed, 2, taida_str_hash((taida_val)"method"));
            taida_pack_set(parsed, 2, taida_net_make_span(0, 0));
            taida_pack_set_tag(parsed, 2, TAIDA_TAG_PACK);
            taida_pack_set_hash(parsed, 3, taida_str_hash((taida_val)"path"));
            taida_pack_set(parsed, 3, taida_net_make_span(0, 0));
            taida_pack_set_tag(parsed, 3, TAIDA_TAG_PACK);
            taida_pack_set_hash(parsed, 4, taida_str_hash((taida_val)"query"));
            taida_pack_set(parsed, 4, taida_net_make_span(0, 0));
            taida_pack_set_tag(parsed, 4, TAIDA_TAG_PACK);
            taida_val ver = taida_pack_new(2);
            taida_pack_set_hash(ver, 0, taida_str_hash((taida_val)"major"));
            taida_pack_set(ver, 0, 1);
            taida_pack_set_hash(ver, 1, taida_str_hash((taida_val)"minor"));
            taida_pack_set(ver, 1, 1);
            taida_pack_set_hash(parsed, 5, taida_str_hash((taida_val)"version"));
            taida_pack_set(parsed, 5, ver);
            taida_pack_set_tag(parsed, 5, TAIDA_TAG_PACK);
            taida_pack_set_hash(parsed, 6, taida_str_hash((taida_val)"headers"));
            taida_pack_set(parsed, 6, taida_list_new());
            taida_pack_set_tag(parsed, 6, TAIDA_TAG_LIST);
            taida_pack_set_hash(parsed, 7, taida_str_hash((taida_val)"bodyOffset"));
            taida_pack_set(parsed, 7, 0);
            taida_pack_set_hash(parsed, 8, taida_str_hash((taida_val)"contentLength"));
            taida_pack_set(parsed, 8, 0);
            taida_pack_set_hash(parsed, 9, taida_str_hash((taida_val)"chunked"));
            taida_pack_set(parsed, 9, 0);  // false
            taida_pack_set_tag(parsed, 9, TAIDA_TAG_BOOL);
            if (free_data) free(data);
            return taida_net_result_ok(parsed);
        }
        if (free_data) free(data);
        return taida_net_result_fail("ParseError", "Malformed HTTP request: no request line");
    }

    // Parse request line: METHOD SP PATH HTTP/x.y
    // Find first SP
    int method_end = -1;
    for (int i = 0; i < first_crlf; i++) {
        if (data[i] == ' ') { method_end = i; break; }
    }
    if (method_end <= 0) {
        if (free_data) free(data);
        return taida_net_result_fail("ParseError", "Malformed HTTP request: invalid request line");
    }

    // Find last SP (before HTTP/x.y)
    int version_start = -1;
    for (int i = first_crlf - 1; i > method_end; i--) {
        if (data[i - 1] == ' ') { version_start = i; break; }
    }
    if (version_start < 0 || version_start <= method_end + 1) {
        if (free_data) free(data);
        return taida_net_result_fail("ParseError", "Malformed HTTP request: invalid request line");
    }

    // Parse version: must be exactly "HTTP/x.y" where x,y are single ASCII digits
    // Strict: reject HTTP/a.b, HTTP/12.34, HTTP/1, etc. (parity with Interpreter/JS)
    int http_major = 1, http_minor = 1;
    int version_len = first_crlf - version_start;
    if (version_len == 8 &&
        data[version_start]   == 'H' && data[version_start+1] == 'T' &&
        data[version_start+2] == 'T' && data[version_start+3] == 'P' &&
        data[version_start+4] == '/' &&
        data[version_start+5] >= '0' && data[version_start+5] <= '9' &&
        data[version_start+6] == '.' &&
        data[version_start+7] >= '0' && data[version_start+7] <= '9') {
        http_major = data[version_start+5] - '0';
        http_minor = data[version_start+7] - '0';
        // NB-32: restrict to HTTP/1.0 and HTTP/1.1 only (parity with Interpreter/httparse)
        // Reject immediately once version is fully parsed, regardless of head completeness
        if (http_major != 1 || (http_minor != 0 && http_minor != 1)) {
            if (free_data) free(data);
            return taida_net_result_fail("ParseError", "Malformed HTTP request: invalid HTTP version");
        }
    } else if (complete) {
        if (free_data) free(data);
        return taida_net_result_fail("ParseError", "Malformed HTTP request: invalid HTTP version");
    }

    // Method span
    int method_start_idx = 0;
    int method_len = method_end;

    // Path + query: between first SP and last SP
    int uri_start = method_end + 1;
    int uri_end = version_start - 1;
    int uri_len = uri_end - uri_start;

    // Split path and query on '?'
    int path_start_idx = uri_start;
    int path_len = uri_len;
    int query_start_idx = 0;
    int query_len = 0;
    for (int i = uri_start; i < uri_end; i++) {
        if (data[i] == '?') {
            path_len = i - uri_start;
            query_start_idx = i + 1;
            query_len = uri_end - (i + 1);
            break;
        }
    }

    // Parse headers
    taida_val headers_list = taida_list_new();
    int64_t content_length = 0;
    int cl_count = 0;
    int has_te_chunked = 0;
    size_t pos = (size_t)(first_crlf + 2);  // after first \r\n

    int header_count = 0;
    while (pos < scan_limit) {
        // Find next \r\n
        size_t line_end = scan_limit;
        for (size_t j = pos; j + 1 < scan_limit; j++) {
            if (data[j] == '\r' && data[j+1] == '\n') {
                line_end = j;
                break;
            }
        }
        if (line_end == pos) break;  // empty line = end of headers

        // NB-4/NB-6: enforce max 64 headers (parity with Interpreter/httparse)
        header_count++;
        if (header_count > 64) {
            if (free_data) free(data);
            return taida_net_result_fail("ParseError", "Malformed HTTP request: too many headers");
        }

        // Find colon separator
        size_t colon = line_end;
        for (size_t j = pos; j < line_end; j++) {
            if (data[j] == ':') { colon = j; break; }
        }
        if (colon >= line_end) {
            // No colon found: if head is complete this is malformed, otherwise incomplete
            if (complete) {
                if (free_data) free(data);
                return taida_net_result_fail("ParseError", "Malformed HTTP request: invalid header line");
            }
            break;  // incomplete — stop parsing headers
        }

        // Header name: pos..colon, value: after colon + OWS trimming
        size_t name_start = pos;
        size_t name_len = colon - pos;
        size_t val_start = colon + 1;
        // NB-34: Skip leading SP/HT and trim trailing SP/HT (parity with Interpreter/httparse)
        while (val_start < line_end && (data[val_start] == ' ' || data[val_start] == '\t')) val_start++;
        size_t val_end = line_end;
        while (val_end > val_start && (data[val_end - 1] == ' ' || data[val_end - 1] == '\t')) val_end--;
        size_t val_len = val_end - val_start;

        taida_val header_pack = taida_pack_new(2);
        taida_pack_set_hash(header_pack, 0, taida_str_hash((taida_val)"name"));
        taida_pack_set(header_pack, 0, taida_net_make_span((taida_val)name_start, (taida_val)name_len));
        taida_pack_set_tag(header_pack, 0, TAIDA_TAG_PACK);
        taida_pack_set_hash(header_pack, 1, taida_str_hash((taida_val)"value"));
        taida_pack_set(header_pack, 1, taida_net_make_span((taida_val)val_start, (taida_val)val_len));
        taida_pack_set_tag(header_pack, 1, TAIDA_TAG_PACK);
        headers_list = taida_list_push(headers_list, header_pack);

        // Check Content-Length (case-insensitive)
        if (name_len == 14) {
            // Check "content-length" case-insensitively
            const char *cl_expected = "content-length";
            int is_cl = 1;
            for (size_t k = 0; k < 14; k++) {
                char c = (char)data[name_start + k];
                if (c >= 'A' && c <= 'Z') c += 32;
                if (c != cl_expected[k]) { is_cl = 0; break; }
            }
            if (is_cl) {
                cl_count++;
                if (cl_count > 1) {
                    if (free_data) free(data);
                    return taida_net_result_fail("ParseError", "Malformed HTTP request: duplicate Content-Length header");
                }
                // Validate: trimmed value must be all digits
                // val_start..val_start+val_len (already trimmed leading spaces/tabs)
                // Also trim trailing spaces and tabs (parity with Interpreter's .trim() and JS's .trim())
                size_t cl_end = val_start + val_len;
                while (cl_end > val_start && (data[cl_end-1] == ' ' || data[cl_end-1] == '\t')) cl_end--;
                size_t cl_len = cl_end - val_start;
                if (cl_len == 0) {
                    if (free_data) free(data);
                    return taida_net_result_fail("ParseError", "Malformed HTTP request: invalid Content-Length value");
                }
                int all_digits = 1;
                for (size_t k = 0; k < cl_len; k++) {
                    if (data[val_start + k] < '0' || data[val_start + k] > '9') {
                        all_digits = 0;
                        break;
                    }
                }
                if (!all_digits) {
                    if (free_data) free(data);
                    return taida_net_result_fail("ParseError", "Malformed HTTP request: invalid Content-Length value");
                }
                // Parse digits
                int64_t cl_val = 0;
                for (size_t k = 0; k < cl_len; k++) {
                    int64_t digit = data[val_start + k] - '0';
                    // Overflow check
                    if (cl_val > (9007199254740991LL - digit) / 10) {
                        if (free_data) free(data);
                        return taida_net_result_fail("ParseError", "Malformed HTTP request: invalid Content-Length value");
                    }
                    cl_val = cl_val * 10 + digit;
                }
                // Cap at Number.MAX_SAFE_INTEGER
                if (cl_val > 9007199254740991LL) {
                    if (free_data) free(data);
                    return taida_net_result_fail("ParseError", "Malformed HTTP request: invalid Content-Length value");
                }
                content_length = cl_val;
            }
        }

        // NET2-2a: Detect Transfer-Encoding: chunked (parity with Interpreter)
        if (name_len == 17) {
            const char *te_expected = "transfer-encoding";
            int is_te = 1;
            for (size_t k = 0; k < 17; k++) {
                char c = (char)data[name_start + k];
                if (c >= 'A' && c <= 'Z') c += 32;
                if (c != te_expected[k]) { is_te = 0; break; }
            }
            if (is_te) {
                // Scan comma-separated tokens for "chunked" (case-insensitive)
                size_t tk_start = val_start;
                while (tk_start < val_start + val_len) {
                    // Skip leading whitespace
                    while (tk_start < val_start + val_len && (data[tk_start] == ' ' || data[tk_start] == '\t')) tk_start++;
                    // Find comma or end
                    size_t tk_end = tk_start;
                    while (tk_end < val_start + val_len && data[tk_end] != ',') tk_end++;
                    // Trim trailing whitespace of token
                    size_t tk_trim = tk_end;
                    while (tk_trim > tk_start && (data[tk_trim - 1] == ' ' || data[tk_trim - 1] == '\t')) tk_trim--;
                    size_t tk_len = tk_trim - tk_start;
                    if (tk_len == 7) {
                        const char *chunked_str = "chunked";
                        int match = 1;
                        for (size_t m = 0; m < 7; m++) {
                            char c = (char)data[tk_start + m];
                            if (c >= 'A' && c <= 'Z') c += 32;
                            if (c != chunked_str[m]) { match = 0; break; }
                        }
                        if (match) has_te_chunked = 1;
                    }
                    tk_start = tk_end + 1;  // skip comma
                }
            }
        }

        pos = line_end + 2;  // skip \r\n
    }

    // NET2-2e: Reject Content-Length + Transfer-Encoding: chunked (RFC 7230 section 3.3.3)
    if (has_te_chunked && cl_count > 0) {
        if (free_data) free(data);
        return taida_net_result_fail("ParseError", "Malformed HTTP request: Content-Length and Transfer-Encoding: chunked are mutually exclusive");
    }

    // Build result pack
    taida_val parsed = taida_pack_new(10);
    taida_pack_set_hash(parsed, 0, taida_str_hash((taida_val)"complete"));
    taida_pack_set(parsed, 0, complete ? 1 : 0);
    taida_pack_set_tag(parsed, 0, TAIDA_TAG_BOOL);
    taida_pack_set_hash(parsed, 1, taida_str_hash((taida_val)"consumed"));
    taida_pack_set(parsed, 1, (taida_val)consumed);
    taida_pack_set_hash(parsed, 2, taida_str_hash((taida_val)"method"));
    taida_pack_set(parsed, 2, taida_net_make_span((taida_val)method_start_idx, (taida_val)method_len));
    taida_pack_set_tag(parsed, 2, TAIDA_TAG_PACK);
    taida_pack_set_hash(parsed, 3, taida_str_hash((taida_val)"path"));
    taida_pack_set(parsed, 3, taida_net_make_span((taida_val)path_start_idx, (taida_val)path_len));
    taida_pack_set_tag(parsed, 3, TAIDA_TAG_PACK);
    taida_pack_set_hash(parsed, 4, taida_str_hash((taida_val)"query"));
    taida_pack_set(parsed, 4, taida_net_make_span((taida_val)query_start_idx, (taida_val)query_len));
    taida_pack_set_tag(parsed, 4, TAIDA_TAG_PACK);

    taida_val ver = taida_pack_new(2);
    taida_pack_set_hash(ver, 0, taida_str_hash((taida_val)"major"));
    taida_pack_set(ver, 0, (taida_val)http_major);
    taida_pack_set_hash(ver, 1, taida_str_hash((taida_val)"minor"));
    taida_pack_set(ver, 1, (taida_val)http_minor);
    taida_pack_set_hash(parsed, 5, taida_str_hash((taida_val)"version"));
    taida_pack_set(parsed, 5, ver);
    taida_pack_set_tag(parsed, 5, TAIDA_TAG_PACK);

    taida_pack_set_hash(parsed, 6, taida_str_hash((taida_val)"headers"));
    taida_pack_set(parsed, 6, headers_list);
    taida_pack_set_tag(parsed, 6, TAIDA_TAG_LIST);

    taida_pack_set_hash(parsed, 7, taida_str_hash((taida_val)"bodyOffset"));
    taida_pack_set(parsed, 7, (taida_val)consumed);

    taida_pack_set_hash(parsed, 8, taida_str_hash((taida_val)"contentLength"));
    taida_pack_set(parsed, 8, (taida_val)content_length);

    taida_pack_set_hash(parsed, 9, taida_str_hash((taida_val)"chunked"));
    taida_pack_set(parsed, 9, has_te_chunked ? 1 : 0);
    taida_pack_set_tag(parsed, 9, TAIDA_TAG_BOOL);

    if (free_data) free(data);
    return taida_net_result_ok(parsed);
}

// ── httpEncodeResponse(response) ────────────────────────────────
// Encode response @(status, headers, body) into HTTP/1.1 wire bytes.
// Returns Result[@(bytes: Bytes), _]
taida_val taida_net_http_encode_response(taida_val response) {
    if (!taida_is_buchi_pack(response)) {
        return taida_net_result_fail("EncodeError", "httpEncodeResponse: argument must be a BuchiPack @(...)");
    }

    // Extract status (required, must be Int in 100-999)
    taida_val status_hash = taida_str_hash((taida_val)"status");
    if (!taida_pack_has_hash(response, status_hash)) {
        return taida_net_result_fail("EncodeError", "httpEncodeResponse: missing required field 'status'");
    }
    taida_val status = taida_pack_get(response, status_hash);
    // NB-14: Type check via field tag — status must be Int.
    // When tag is UNKNOWN, resolve via runtime detection to catch non-Int values
    // that the compiler couldn't type-check statically.
    {
        taida_val status_tag = taida_pack_get_field_tag(response, status_hash);
        if (status_tag == TAIDA_TAG_UNKNOWN) {
            status_tag = taida_runtime_detect_tag(status);
        }
        if (status_tag != TAIDA_TAG_INT) {
            char val_buf[64];
            taida_format_value(status_tag, status, val_buf, sizeof(val_buf));
            char err_msg[128];
            snprintf(err_msg, sizeof(err_msg),
                     "httpEncodeResponse: status must be Int, got %s",
                     val_buf);
            return taida_net_result_fail("EncodeError", err_msg);
        }
    }
    if (status < 100 || status > 999) {
        char err_msg[128];
        snprintf(err_msg, sizeof(err_msg), "httpEncodeResponse: status must be 100-999, got %d", (int)status);
        return taida_net_result_fail("EncodeError", err_msg);
    }

    // Extract headers (required, must be a List)
    taida_val headers_hash = taida_str_hash((taida_val)"headers");
    if (!taida_pack_has_hash(response, headers_hash)) {
        return taida_net_result_fail("EncodeError", "httpEncodeResponse: missing required field 'headers'");
    }
    taida_val headers_ptr = taida_pack_get(response, headers_hash);
    if (!taida_is_list(headers_ptr)) {
        // NB-21: Format actual value for parity with Interpreter/JS
        taida_val htag = taida_pack_get_field_tag(response, headers_hash);
        char val_buf[64];
        taida_format_value(htag, headers_ptr, val_buf, sizeof(val_buf));
        char err_msg[128];
        snprintf(err_msg, sizeof(err_msg), "httpEncodeResponse: headers must be a List, got %s",
                 val_buf);
        return taida_net_result_fail("EncodeError", err_msg);
    }

    // Extract body (required, must be Bytes or Str)
    // NB6-4: For Bytes, defer materialization until the wire buffer is ready.
    // Instead of allocating a separate body_data buffer and copying twice
    // (taida_val -> body_data -> wire buf), we record the source pointer and
    // copy directly into the wire buffer once.
    taida_val body_hash = taida_str_hash((taida_val)"body");
    if (!taida_pack_has_hash(response, body_hash)) {
        return taida_net_result_fail("EncodeError", "httpEncodeResponse: missing required field 'body'");
    }
    taida_val body_ptr = taida_pack_get(response, body_hash);
    unsigned char *body_data = NULL;  // contiguous body (Str path only)
    taida_val *body_bytes_arr = NULL; // taida_val array (Bytes path only)
    size_t body_len = 0;
    int body_is_bytes = 0;

    if (TAIDA_IS_BYTES(body_ptr)) {
        body_bytes_arr = (taida_val*)body_ptr;
        taida_val blen = body_bytes_arr[1];
        if (blen < 0) blen = 0;
        body_len = (size_t)blen;
        body_is_bytes = 1;
    } else {
        size_t slen = 0;
        if (taida_read_cstr_len_safe((const char*)body_ptr, 10485760, &slen)) {
            body_data = (unsigned char*)body_ptr;
            body_len = slen;
        } else {
            // NB-21: Format actual value for parity with Interpreter/JS
            taida_val btag = taida_pack_get_field_tag(response, body_hash);
            char val_buf[64];
            taida_format_value(btag, body_ptr, val_buf, sizeof(val_buf));
            char err_msg[128];
            snprintf(err_msg, sizeof(err_msg), "httpEncodeResponse: body must be Bytes or Str, got %s",
                     val_buf);
            return taida_net_result_fail("EncodeError", err_msg);
        }
    }

    // RFC 9110: 1xx, 204, 205, 304 MUST NOT contain a message body
    int no_body = (status >= 100 && status < 200) || status == 204 || status == 205 || status == 304;
    if (no_body && body_len > 0) {
        char err_msg[128];
        snprintf(err_msg, sizeof(err_msg), "httpEncodeResponse: status %d must not have a body", (int)status);
        return taida_net_result_fail("EncodeError", err_msg);
    }

    // Build HTTP response buffer
    size_t buf_cap = 512 + body_len;
    unsigned char *buf = (unsigned char*)TAIDA_MALLOC(buf_cap, "net_encode_buf");
    size_t buf_len = 0;

    // Status line
    const char *reason = taida_net_status_reason((int)status);
    buf_len += (size_t)snprintf((char*)buf + buf_len, buf_cap - buf_len,
                                 "HTTP/1.1 %d %s\r\n", (int)status, reason);

    // User headers
    int has_content_length = 0;
    taida_val name_hash = taida_str_hash((taida_val)"name");
    taida_val value_hash = taida_str_hash((taida_val)"value");

    {
        taida_val *hlist = (taida_val*)headers_ptr;
        taida_val hcount = hlist[2];
        for (taida_val i = 0; i < hcount; i++) {
            taida_val hdr = hlist[4 + i];
            if (!taida_is_buchi_pack(hdr)) {
                free(buf);
                char err_msg[128];
                snprintf(err_msg, sizeof(err_msg), "httpEncodeResponse: headers[%d] must be @(name, value)", (int)i);
                return taida_net_result_fail("EncodeError", err_msg);
            }
            taida_val hname = taida_pack_get(hdr, name_hash);
            taida_val hvalue = taida_pack_get(hdr, value_hash);
            const char *hname_s = (const char*)hname;
            const char *hvalue_s = (const char*)hvalue;
            size_t hn_len = 0, hv_len = 0;
            if (!taida_read_cstr_len_safe(hname_s, 8192, &hn_len)) {
                free(buf);
                char err_msg[128];
                snprintf(err_msg, sizeof(err_msg), "httpEncodeResponse: headers[%d].name must be Str", (int)i);
                return taida_net_result_fail("EncodeError", err_msg);
            }
            if (!taida_read_cstr_len_safe(hvalue_s, 65536, &hv_len)) {
                free(buf);
                char err_msg[128];
                snprintf(err_msg, sizeof(err_msg), "httpEncodeResponse: headers[%d].value must be Str", (int)i);
                return taida_net_result_fail("EncodeError", err_msg);
            }

            // NB-13: Check for CRLF injection with index + name/value distinction (parity with Interpreter/JS)
            for (size_t k = 0; k < hn_len; k++) {
                if (hname_s[k] == '\r' || hname_s[k] == '\n') {
                    free(buf);
                    char err_msg[128];
                    snprintf(err_msg, sizeof(err_msg), "httpEncodeResponse: headers[%d].name contains CR/LF", (int)i);
                    return taida_net_result_fail("EncodeError", err_msg);
                }
            }
            for (size_t k = 0; k < hv_len; k++) {
                if (hvalue_s[k] == '\r' || hvalue_s[k] == '\n') {
                    free(buf);
                    char err_msg[128];
                    snprintf(err_msg, sizeof(err_msg), "httpEncodeResponse: headers[%d].value contains CR/LF", (int)i);
                    return taida_net_result_fail("EncodeError", err_msg);
                }
            }

            // Skip Content-Length for no-body statuses
            if (no_body && hn_len == 14) {
                const char *cl_expected = "content-length";
                int is_cl = 1;
                for (size_t k = 0; k < 14; k++) {
                    char c = hname_s[k];
                    if (c >= 'A' && c <= 'Z') c += 32;
                    if (c != cl_expected[k]) { is_cl = 0; break; }
                }
                if (is_cl) continue;
            }

            // Check if user provided Content-Length
            if (hn_len == 14) {
                const char *cl_expected = "content-length";
                int is_cl = 1;
                for (size_t k = 0; k < 14; k++) {
                    char c = hname_s[k];
                    if (c >= 'A' && c <= 'Z') c += 32;
                    if (c != cl_expected[k]) { is_cl = 0; break; }
                }
                if (is_cl) has_content_length = 1;
            }

            // Grow buffer if needed
            size_t needed = buf_len + hn_len + hv_len + 4;
            if (needed > buf_cap) {
                buf_cap = needed * 2;
                TAIDA_REALLOC(buf, buf_cap, "net_encode_headers");
            }
            memcpy(buf + buf_len, hname_s, hn_len); buf_len += hn_len;
            buf[buf_len++] = ':'; buf[buf_len++] = ' ';
            memcpy(buf + buf_len, hvalue_s, hv_len); buf_len += hv_len;
            buf[buf_len++] = '\r'; buf[buf_len++] = '\n';
        }
    }

    // Auto-append Content-Length for statuses that allow a body
    if (!no_body && !has_content_length) {
        char cl_hdr[64];
        int cl_len = snprintf(cl_hdr, sizeof(cl_hdr), "Content-Length: %zu\r\n", body_len);
        size_t needed = buf_len + (size_t)cl_len;
        if (needed > buf_cap) {
            buf_cap = needed * 2;
            TAIDA_REALLOC(buf, buf_cap, "net_encode_cl");
        }
        memcpy(buf + buf_len, cl_hdr, (size_t)cl_len);
        buf_len += (size_t)cl_len;
    }

    // End of headers
    size_t needed = buf_len + 2 + body_len;
    if (needed > buf_cap) {
        buf_cap = needed;
        TAIDA_REALLOC(buf, buf_cap, "net_encode_body");
    }
    buf[buf_len++] = '\r'; buf[buf_len++] = '\n';

    // NB6-4: Copy body directly into wire buffer — single copy from source.
    // For Bytes: copy from taida_val array directly (no intermediate buffer).
    // For Str: memcpy from C string pointer (already contiguous).
    if (!no_body && body_len > 0) {
        if (body_is_bytes) {
            for (size_t i = 0; i < body_len; i++) {
                buf[buf_len + i] = (unsigned char)body_bytes_arr[2 + i];
            }
        } else {
            memcpy(buf + buf_len, body_data, body_len);
        }
        buf_len += body_len;
    }

    // Convert to Bytes value
    taida_val result_bytes = taida_bytes_from_raw(buf, (taida_val)buf_len);
    free(buf);

    taida_val result = taida_pack_new(1);
    taida_pack_set_hash(result, 0, taida_str_hash((taida_val)"bytes"));
    taida_pack_set(result, 0, result_bytes);
    taida_pack_set_tag(result, 0, TAIDA_TAG_PACK);  // Bytes IS-A tagged ptr

    return taida_net_result_ok(result);
}

// ── net_send_all: short-write safe send helper ──────────────────
// Loops send() until all bytes are written or an error occurs.
// Returns 0 on success, -1 on error.
// NET5-4a: Routes through TLS when tl_ssl is active.
static int taida_net_send_all(int fd, const void *buf, size_t len) {
    return taida_tls_send_all(fd, buf, len);
}

// ── readBody(req) → Bytes ────────────────────────────────────────
// Extract body bytes from a request pack.
// req.raw (Bytes) + body span (start, len) → body slice as new Bytes.
// If body.len == 0 or body span is absent, returns empty Bytes.
taida_val taida_net_read_body(taida_val req) {
    if (!taida_is_buchi_pack(req)) {
        // Parity: Interpreter returns RuntimeError, JS throws __NativeError
        char val_buf[64];
        taida_val tag = taida_runtime_detect_tag(req);
        taida_format_value(tag, req, val_buf, sizeof(val_buf));
        char err_msg[256];
        snprintf(err_msg, sizeof(err_msg),
                 "readBody: argument must be a request pack @(...), got %s",
                 val_buf);
        return taida_throw(taida_make_error("TypeError", err_msg));
    }

    // v4: If the request has __body_stream sentinel (2-arg handler),
    // delegate to readBodyAll to stream from socket.
    if (taida_net4_is_body_stream_request(req)) {
        return taida_net_read_body_all(req);
    }

    // Extract raw: Bytes
    taida_val raw = taida_pack_get(req, taida_str_hash((taida_val)"raw"));
    if (!TAIDA_IS_BYTES(raw)) {
        return taida_throw(taida_make_error("TypeError",
            "readBody: request pack missing 'raw: Bytes' field"));
    }

    // Extract body: @(start: Int, len: Int)
    taida_val body_span = taida_pack_get(req, taida_str_hash((taida_val)"body"));
    taida_val body_start = 0;
    taida_val body_len = 0;
    if (body_span != 0 && taida_is_buchi_pack(body_span)) {
        body_start = taida_pack_get(body_span, taida_str_hash((taida_val)"start"));
        body_len = taida_pack_get(body_span, taida_str_hash((taida_val)"len"));
    }

    if (body_len <= 0) {
        return taida_bytes_new_filled(0, 0);
    }

    // raw layout: [magic+rc, length, b0, b1, ...]
    taida_val *raw_arr = (taida_val*)raw;
    taida_val raw_len = raw_arr[1];

    // Clamp to valid range
    if (body_start < 0) body_start = 0;
    if (body_start > raw_len) body_start = raw_len;
    taida_val end = body_start + body_len;
    if (end > raw_len) end = raw_len;
    taida_val actual_len = end - body_start;
    if (actual_len <= 0) {
        return taida_bytes_new_filled(0, 0);
    }

    // Copy body bytes into a new Bytes object
    taida_val out = taida_bytes_new_filled(actual_len, 0);
    taida_val *out_arr = (taida_val*)out;
    for (taida_val i = 0; i < actual_len; i++) {
        out_arr[2 + i] = raw_arr[2 + body_start + i];
    }
    return out;
}

// ── NET2-5a: Keep-Alive determination ──────────────────────────
// Determine whether the connection should be kept alive based on
// HTTP version and the Connection header value.
// Rules (RFC 7230 S6.1):
//   HTTP/1.1: keep-alive by default, Connection: close disables
//   HTTP/1.0: close by default, Connection: keep-alive enables
// raw is the wire bytes buffer, headers is the parsed header list.
static int taida_net_determine_keep_alive(
    const unsigned char *raw, size_t raw_len,
    taida_val headers, taida_val http_minor
) {
    int has_close = 0;
    int has_keep_alive = 0;

    if (!TAIDA_IS_LIST(headers)) {
        return (http_minor == 1) ? 1 : 0;
    }

    taida_val *hdr_list = (taida_val*)headers;
    taida_val hdr_count = hdr_list[2];  // list length at index 2 (layout: [magic+rc, capacity, length, elem_tag, ...])

    for (taida_val i = 0; i < hdr_count; i++) {
        taida_val header = hdr_list[4 + i];
        if (!taida_is_buchi_pack(header)) continue;

        // Get name span
        taida_val name_span = taida_pack_get(header, taida_str_hash((taida_val)"name"));
        if (!taida_is_buchi_pack(name_span)) continue;
        taida_val name_start = taida_pack_get(name_span, taida_str_hash((taida_val)"start"));
        taida_val name_len = taida_pack_get(name_span, taida_str_hash((taida_val)"len"));
        if (name_start < 0 || name_len <= 0) continue;
        if ((size_t)(name_start + name_len) > raw_len) continue;

        // Case-insensitive compare with "connection" (10 chars)
        if (name_len != 10) continue;
        const char *conn_str = "connection";
        int match = 1;
        for (int j = 0; j < 10; j++) {
            char c = (char)raw[name_start + j];
            if (c >= 'A' && c <= 'Z') c += 32;
            if (c != conn_str[j]) { match = 0; break; }
        }
        if (!match) continue;

        // Extract value span and scan comma-separated tokens
        taida_val val_span = taida_pack_get(header, taida_str_hash((taida_val)"value"));
        if (!taida_is_buchi_pack(val_span)) continue;
        taida_val val_start = taida_pack_get(val_span, taida_str_hash((taida_val)"start"));
        taida_val val_len = taida_pack_get(val_span, taida_str_hash((taida_val)"len"));
        if (val_start < 0 || val_len <= 0) continue;
        if ((size_t)(val_start + val_len) > raw_len) continue;

        // Scan tokens split by ','
        const unsigned char *vp = raw + val_start;
        size_t vl = (size_t)val_len;
        size_t tok_start = 0;
        for (size_t k = 0; k <= vl; k++) {
            if (k == vl || vp[k] == ',') {
                // Trim whitespace
                size_t ts = tok_start, te = k;
                while (ts < te && (vp[ts] == ' ' || vp[ts] == '\t')) ts++;
                while (te > ts && (vp[te-1] == ' ' || vp[te-1] == '\t')) te--;
                size_t tlen = te - ts;
                if (tlen == 5) {
                    // "close"
                    int mc = 1;
                    const char *cs = "close";
                    for (size_t m = 0; m < 5; m++) {
                        char c = (char)vp[ts + m];
                        if (c >= 'A' && c <= 'Z') c += 32;
                        if (c != cs[m]) { mc = 0; break; }
                    }
                    if (mc) has_close = 1;
                } else if (tlen == 10) {
                    // "keep-alive"
                    int mk = 1;
                    const char *ks = "keep-alive";
                    for (size_t m = 0; m < 10; m++) {
                        char c = (char)vp[ts + m];
                        if (c >= 'A' && c <= 'Z') c += 32;
                        if (c != ks[m]) { mk = 0; break; }
                    }
                    if (mk) has_keep_alive = 1;
                }
                tok_start = k + 1;
            }
        }
        // Don't break — merge multiple Connection headers
    }

    // RFC 7230 S6.1: close always wins
    if (has_close) return 0;
    if (http_minor == 1) return 1;  // HTTP/1.1 default keep-alive
    return has_keep_alive ? 1 : 0;  // HTTP/1.0 default close
}

// ── NET2-5b: Chunked in-place compaction ────────────────────────
// Result struct for chunked compaction
typedef struct {
    size_t body_len;       // compacted body length
    size_t wire_consumed;  // total bytes consumed from body_offset
} ChunkedCompactResult;

// Find the first CRLF in buf[0..len). Returns offset of '\r', or -1 if not found.
static int64_t taida_net_find_crlf(const unsigned char *data, size_t len) {
    if (len < 2) return -1;
    for (size_t i = 0; i + 1 < len; i++) {
        if (data[i] == '\r' && data[i + 1] == '\n') return (int64_t)i;
    }
    return -1;
}

// Check if a complete chunked body is available (read-only scan).
// Returns wire_consumed on success, -1 if incomplete, -2 if malformed.
static int64_t taida_net_chunked_body_complete(
    const unsigned char *buf, size_t total_len, size_t body_offset
) {
    size_t data_len = total_len - body_offset;
    size_t rp = 0;

    for (;;) {
        if (rp >= data_len) return -1; // incomplete

        int64_t crlf = taida_net_find_crlf(buf + body_offset + rp, data_len - rp);
        if (crlf < 0) return -1; // incomplete

        // Parse hex chunk-size, ignoring chunk-ext after ';'
        size_t hex_end = (size_t)crlf;
        for (size_t i = 0; i < hex_end; i++) {
            if (buf[body_offset + rp + i] == ';') { hex_end = i; break; }
        }
        // Trim whitespace
        size_t hs = 0, he = hex_end;
        while (hs < he && (buf[body_offset + rp + hs] == ' ' || buf[body_offset + rp + hs] == '\t')) hs++;
        while (he > hs && (buf[body_offset + rp + he - 1] == ' ' || buf[body_offset + rp + he - 1] == '\t')) he--;
        if (hs >= he) return -2; // empty chunk-size = malformed

        // Parse hex
        // NB2-5: Reject chunk-size with more than 15 hex digits (max safe: 0xFFFFFFFFFFFFFFF)
        // to prevent size_t overflow that silently wraps to 0 and accepts malformed input.
        if (he - hs > 15) return -2; // oversized chunk-size = malformed
        size_t chunk_size = 0;
        for (size_t i = hs; i < he; i++) {
            unsigned char c = buf[body_offset + rp + i];
            int digit = -1;
            if (c >= '0' && c <= '9') digit = c - '0';
            else if (c >= 'a' && c <= 'f') digit = 10 + c - 'a';
            else if (c >= 'A' && c <= 'F') digit = 10 + c - 'A';
            if (digit < 0) return -2; // invalid hex
            chunk_size = chunk_size * 16 + (size_t)digit;
        }

        rp += (size_t)crlf + 2; // skip "chunk-size\r\n"

        if (chunk_size == 0) {
            // Terminator chunk: skip trailers
            for (;;) {
                if (rp + 2 > data_len) return -1; // incomplete
                if (buf[body_offset + rp] == '\r' && buf[body_offset + rp + 1] == '\n') {
                    rp += 2;
                    return (int64_t)rp;
                }
                int64_t tc = taida_net_find_crlf(buf + body_offset + rp, data_len - rp);
                if (tc < 0) return -1; // incomplete
                rp += (size_t)tc + 2;
            }
        }

        // Check data + CRLF
        if (rp + chunk_size + 2 > data_len) return -1; // incomplete
        rp += chunk_size;
        if (buf[body_offset + rp] != '\r' || buf[body_offset + rp + 1] != '\n') return -2; // malformed
        rp += 2;
    }
}

// In-place compaction: remove chunk framing, compact data in-place using memmove.
// Returns 0 on success (result written to *out), -1 on error.
static int taida_net_chunked_in_place_compact(
    unsigned char *buf, size_t body_offset, ChunkedCompactResult *out
) {
    size_t rp = 0; // read position relative to body_offset
    size_t wp = 0; // write position relative to body_offset

    for (;;) {
        int64_t crlf = taida_net_find_crlf(buf + body_offset + rp, 1048576);
        if (crlf < 0) return -1;

        // Parse hex chunk-size, ignoring chunk-ext
        size_t hex_end = (size_t)crlf;
        for (size_t i = 0; i < hex_end; i++) {
            if (buf[body_offset + rp + i] == ';') { hex_end = i; break; }
        }
        size_t hs = 0, he = hex_end;
        while (hs < he && (buf[body_offset + rp + hs] == ' ' || buf[body_offset + rp + hs] == '\t')) hs++;
        while (he > hs && (buf[body_offset + rp + he - 1] == ' ' || buf[body_offset + rp + he - 1] == '\t')) he--;
        if (hs >= he) return -1;

        // NB2-5: Reject oversized chunk-size to prevent overflow (parity with body_complete)
        if (he - hs > 15) return -1;
        size_t chunk_size = 0;
        for (size_t i = hs; i < he; i++) {
            unsigned char c = buf[body_offset + rp + i];
            int digit = -1;
            if (c >= '0' && c <= '9') digit = c - '0';
            else if (c >= 'a' && c <= 'f') digit = 10 + c - 'a';
            else if (c >= 'A' && c <= 'F') digit = 10 + c - 'A';
            if (digit < 0) return -1;
            chunk_size = chunk_size * 16 + (size_t)digit;
        }

        rp += (size_t)crlf + 2; // skip "size\r\n"

        if (chunk_size == 0) {
            // Skip trailers until final CRLF
            for (;;) {
                if (buf[body_offset + rp] == '\r' && buf[body_offset + rp + 1] == '\n') {
                    rp += 2;
                    break;
                }
                int64_t tc = taida_net_find_crlf(buf + body_offset + rp, 1048576);
                if (tc < 0) return -1;
                rp += (size_t)tc + 2;
            }
            out->body_len = wp;
            out->wire_consumed = rp;
            return 0;
        }

        // In-place copy using memmove (safe for overlapping regions)
        if (wp != rp) {
            memmove(buf + body_offset + wp, buf + body_offset + rp, chunk_size);
        }
        wp += chunk_size;
        rp += chunk_size;

        // Validate trailing CRLF
        if (buf[body_offset + rp] != '\r' || buf[body_offset + rp + 1] != '\n') return -1;
        rp += 2;
    }
}

// ── NET2-5: httpServe helper — build request pack ────────────────
static taida_val taida_net_build_request_pack(
    const unsigned char *raw_data, size_t raw_len,
    size_t body_start, size_t body_len, int64_t content_length,
    int is_chunked, int keep_alive,
    const char *remote_host, int remote_port
) {
    taida_val raw_bytes = taida_bytes_from_raw(raw_data, (taida_val)raw_len);

    // Parse to get spans
    taida_val parse_result = taida_net_http_parse_request_head(raw_bytes);
    taida_val inner = taida_pack_get(parse_result, taida_str_hash((taida_val)"__value"));

    taida_val request = taida_pack_new(13);
    taida_pack_set_hash(request, 0, taida_str_hash((taida_val)"raw"));
    taida_pack_set(request, 0, raw_bytes);
    taida_pack_set_tag(request, 0, TAIDA_TAG_PACK);  // Bytes
    taida_retain(raw_bytes);

    if (inner != 0 && taida_is_buchi_pack(inner)) {
        taida_val method_v = taida_pack_get(inner, taida_str_hash((taida_val)"method"));
        taida_pack_set_hash(request, 1, taida_str_hash((taida_val)"method"));
        taida_pack_set(request, 1, method_v);
        taida_pack_set_tag(request, 1, TAIDA_TAG_PACK);
        if (method_v > 4096) taida_retain(method_v);

        taida_val path_v = taida_pack_get(inner, taida_str_hash((taida_val)"path"));
        taida_pack_set_hash(request, 2, taida_str_hash((taida_val)"path"));
        taida_pack_set(request, 2, path_v);
        taida_pack_set_tag(request, 2, TAIDA_TAG_PACK);
        if (path_v > 4096) taida_retain(path_v);

        taida_val query_v = taida_pack_get(inner, taida_str_hash((taida_val)"query"));
        taida_pack_set_hash(request, 3, taida_str_hash((taida_val)"query"));
        taida_pack_set(request, 3, query_v);
        taida_pack_set_tag(request, 3, TAIDA_TAG_PACK);
        if (query_v > 4096) taida_retain(query_v);

        taida_val version_v = taida_pack_get(inner, taida_str_hash((taida_val)"version"));
        taida_pack_set_hash(request, 4, taida_str_hash((taida_val)"version"));
        taida_pack_set(request, 4, version_v);
        taida_pack_set_tag(request, 4, TAIDA_TAG_PACK);
        if (version_v > 4096) taida_retain(version_v);

        taida_val headers_v = taida_pack_get(inner, taida_str_hash((taida_val)"headers"));
        taida_pack_set_hash(request, 5, taida_str_hash((taida_val)"headers"));
        taida_pack_set(request, 5, headers_v);
        taida_pack_set_tag(request, 5, TAIDA_TAG_LIST);
        if (headers_v > 4096) taida_retain(headers_v);
    } else {
        taida_pack_set_hash(request, 1, taida_str_hash((taida_val)"method"));
        taida_pack_set(request, 1, taida_net_make_span(0, 0));
        taida_pack_set_tag(request, 1, TAIDA_TAG_PACK);
        taida_pack_set_hash(request, 2, taida_str_hash((taida_val)"path"));
        taida_pack_set(request, 2, taida_net_make_span(0, 0));
        taida_pack_set_tag(request, 2, TAIDA_TAG_PACK);
        taida_pack_set_hash(request, 3, taida_str_hash((taida_val)"query"));
        taida_pack_set(request, 3, taida_net_make_span(0, 0));
        taida_pack_set_tag(request, 3, TAIDA_TAG_PACK);
        taida_val ver = taida_pack_new(2);
        taida_pack_set_hash(ver, 0, taida_str_hash((taida_val)"major"));
        taida_pack_set(ver, 0, 1);
        taida_pack_set_hash(ver, 1, taida_str_hash((taida_val)"minor"));
        taida_pack_set(ver, 1, 1);
        taida_pack_set_hash(request, 4, taida_str_hash((taida_val)"version"));
        taida_pack_set(request, 4, ver);
        taida_pack_set_tag(request, 4, TAIDA_TAG_PACK);
        taida_pack_set_hash(request, 5, taida_str_hash((taida_val)"headers"));
        taida_pack_set(request, 5, taida_list_new());
        taida_pack_set_tag(request, 5, TAIDA_TAG_LIST);
    }

    taida_pack_set_hash(request, 6, taida_str_hash((taida_val)"body"));
    taida_pack_set(request, 6, taida_net_make_span((taida_val)body_start, (taida_val)body_len));
    taida_pack_set_tag(request, 6, TAIDA_TAG_PACK);

    taida_pack_set_hash(request, 7, taida_str_hash((taida_val)"bodyOffset"));
    taida_pack_set(request, 7, (taida_val)body_start);

    taida_pack_set_hash(request, 8, taida_str_hash((taida_val)"contentLength"));
    taida_pack_set(request, 8, (taida_val)content_length);

    taida_pack_set_hash(request, 9, taida_str_hash((taida_val)"remoteHost"));
    taida_pack_set(request, 9, (taida_val)taida_str_new_copy(remote_host));
    taida_pack_set_tag(request, 9, TAIDA_TAG_STR);

    taida_pack_set_hash(request, 10, taida_str_hash((taida_val)"remotePort"));
    taida_pack_set(request, 10, (taida_val)remote_port);

    taida_pack_set_hash(request, 11, taida_str_hash((taida_val)"keepAlive"));
    taida_pack_set(request, 11, keep_alive ? 1 : 0);
    taida_pack_set_tag(request, 11, TAIDA_TAG_BOOL);

    taida_pack_set_hash(request, 12, taida_str_hash((taida_val)"chunked"));
    taida_pack_set(request, 12, is_chunked ? 1 : 0);
    taida_pack_set_tag(request, 12, TAIDA_TAG_BOOL);

    return request;
}

// ── NET2-5: httpServe helper — send encoded response ─────────────
// NB2-20: Send directly from Bytes internal array — no extra malloc + byte-by-byte copy.
// Bytes layout: [header(magic+rc), length, byte0, byte1, ...] — each byte is a taida_val.
// We still need a contiguous buffer because taida_val slots are 8 bytes each (not 1 byte).
// Optimization: use stack buffer for small responses, heap only for large ones.
static void taida_net_send_response(int client_fd, taida_val encoded) {
    taida_val enc_throw = taida_pack_get(encoded, taida_str_hash((taida_val)"throw"));
    if (enc_throw == 0) {
        taida_val enc_inner = taida_pack_get(encoded, taida_str_hash((taida_val)"__value"));
        if (enc_inner != 0 && taida_is_buchi_pack(enc_inner)) {
            taida_val wire_bytes = taida_pack_get(enc_inner, taida_str_hash((taida_val)"bytes"));
            if (TAIDA_IS_BYTES(wire_bytes)) {
                taida_val *wb = (taida_val*)wire_bytes;
                taida_val wb_len = wb[1];
                // Use stack buffer for typical responses (< 4KB), heap for larger
                unsigned char stack_buf[4096];
                unsigned char *wb_buf;
                int heap_alloc = 0;
                if ((size_t)wb_len <= sizeof(stack_buf)) {
                    wb_buf = stack_buf;
                } else {
                    wb_buf = (unsigned char*)TAIDA_MALLOC((size_t)wb_len, "net_serve_send");
                    heap_alloc = 1;
                }
                for (taida_val i = 0; i < wb_len; i++) wb_buf[i] = (unsigned char)wb[2 + i];
                taida_net_send_all(client_fd, wb_buf, (size_t)wb_len);
                if (heap_alloc) free(wb_buf);
            }
        }
    } else {
        const char *fallback = "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        taida_net_send_all(client_fd, fallback, strlen(fallback));
    }
}

// NB6-1: Scatter-gather send for internal one-shot response path.
// Builds head in one buffer, then sends head + body via writev (2 iovecs).
// Avoids the aggregate buffer concatenation of encode → materialize → send.
// Returns 0 on success, -1 on error.
static int taida_net_send_response_scatter(int client_fd, taida_val response) {
    if (!taida_is_buchi_pack(response)) return -1;

    taida_val status_hash = taida_str_hash((taida_val)"status");
    if (!taida_pack_has_hash(response, status_hash)) return -1;
    taida_val status = taida_pack_get(response, status_hash);
    if (status < 100 || status > 999) return -1;

    taida_val headers_hash = taida_str_hash((taida_val)"headers");
    if (!taida_pack_has_hash(response, headers_hash)) return -1;
    taida_val headers_ptr = taida_pack_get(response, headers_hash);
    if (!taida_is_list(headers_ptr)) return -1;

    taida_val body_hash = taida_str_hash((taida_val)"body");
    if (!taida_pack_has_hash(response, body_hash)) return -1;
    taida_val body_ptr = taida_pack_get(response, body_hash);

    // Determine body source and length.
    const unsigned char *body_data = NULL;
    taida_val *body_bytes_arr = NULL;
    size_t body_len = 0;
    int body_is_bytes = 0;

    if (TAIDA_IS_BYTES(body_ptr)) {
        body_bytes_arr = (taida_val*)body_ptr;
        taida_val blen = body_bytes_arr[1];
        if (blen < 0) blen = 0;
        body_len = (size_t)blen;
        body_is_bytes = 1;
    } else {
        size_t slen = 0;
        if (taida_read_cstr_len_safe((const char*)body_ptr, 10485760, &slen)) {
            body_data = (const unsigned char*)body_ptr;
            body_len = slen;
        } else {
            return -1;
        }
    }

    int no_body = (status >= 100 && status < 200) || status == 204 || status == 205 || status == 304;
    if (no_body && body_len > 0) return -1;

    // Build head buffer.
    char head_stack[2048];
    char *head = head_stack;
    size_t head_cap = sizeof(head_stack);
    size_t head_len = 0;

    const char *reason = taida_net_status_reason((int)status);
    head_len += (size_t)snprintf(head + head_len, head_cap - head_len,
                                  "HTTP/1.1 %d %s\r\n", (int)status, reason);

    taida_val name_hash = taida_str_hash((taida_val)"name");
    taida_val value_hash = taida_str_hash((taida_val)"value");
    int has_content_length = 0;

    taida_val *hlist = (taida_val*)headers_ptr;
    taida_val hcount = hlist[2];
    for (taida_val i = 0; i < hcount; i++) {
        taida_val hdr = hlist[4 + i];
        if (!taida_is_buchi_pack(hdr)) { if (head != head_stack) free(head); return -1; }
        taida_val hname = taida_pack_get(hdr, name_hash);
        taida_val hvalue = taida_pack_get(hdr, value_hash);
        const char *hname_s = (const char*)hname;
        const char *hvalue_s = (const char*)hvalue;
        size_t hn_len = 0, hv_len = 0;
        if (!taida_read_cstr_len_safe(hname_s, 8192, &hn_len)) { if (head != head_stack) free(head); return -1; }
        if (!taida_read_cstr_len_safe(hvalue_s, 65536, &hv_len)) { if (head != head_stack) free(head); return -1; }

        // NB-13: Reject CRLF in header name/value (parity with public encoder)
        for (size_t k = 0; k < hn_len; k++) {
            if (hname_s[k] == '\r' || hname_s[k] == '\n') { if (head != head_stack) free(head); return -1; }
        }
        for (size_t k = 0; k < hv_len; k++) {
            if (hvalue_s[k] == '\r' || hvalue_s[k] == '\n') { if (head != head_stack) free(head); return -1; }
        }

        // Check content-length
        if (hn_len == 14) {
            const char *cl_expected = "content-length";
            int is_cl = 1;
            for (size_t k = 0; k < 14; k++) {
                char c = hname_s[k];
                if (c >= 'A' && c <= 'Z') c += 32;
                if (c != cl_expected[k]) { is_cl = 0; break; }
            }
            if (is_cl) {
                if (no_body) continue;
                has_content_length = 1;
            }
        }

        size_t needed = head_len + hn_len + hv_len + 4;
        if (needed > head_cap) {
            head_cap = needed * 2;
            if (head == head_stack) {
                head = (char*)TAIDA_MALLOC(head_cap, "net_scatter_head");
                memcpy(head, head_stack, head_len);
            } else {
                TAIDA_REALLOC(head, head_cap, "net_scatter_head");
            }
        }
        memcpy(head + head_len, hname_s, hn_len); head_len += hn_len;
        head[head_len++] = ':'; head[head_len++] = ' ';
        memcpy(head + head_len, hvalue_s, hv_len); head_len += hv_len;
        head[head_len++] = '\r'; head[head_len++] = '\n';
    }

    if (!no_body && !has_content_length) {
        char cl_hdr[64];
        int cl_len = snprintf(cl_hdr, sizeof(cl_hdr), "Content-Length: %zu\r\n", body_len);
        size_t needed = head_len + (size_t)cl_len;
        if (needed > head_cap) {
            head_cap = needed * 2;
            if (head == head_stack) {
                head = (char*)TAIDA_MALLOC(head_cap, "net_scatter_head");
                memcpy(head, head_stack, head_len);
            } else {
                TAIDA_REALLOC(head, head_cap, "net_scatter_head");
            }
        }
        memcpy(head + head_len, cl_hdr, (size_t)cl_len);
        head_len += (size_t)cl_len;
    }

    // End of headers.
    if (head_len + 2 > head_cap) {
        head_cap = head_len + 2;
        if (head == head_stack) {
            head = (char*)TAIDA_MALLOC(head_cap, "net_scatter_head");
            memcpy(head, head_stack, head_len);
        } else {
            TAIDA_REALLOC(head, head_cap, "net_scatter_head");
        }
    }
    head[head_len++] = '\r'; head[head_len++] = '\n';

    // Send using scatter-gather (writev).
    int rc;
    if (no_body || body_len == 0) {
        rc = taida_net_send_all(client_fd, head, head_len);
    } else if (!body_is_bytes) {
        // Str body: already contiguous, use 2 iovecs.
        struct iovec iov[2];
        iov[0].iov_base = head;
        iov[0].iov_len = head_len;
        iov[1].iov_base = (void*)body_data;
        iov[1].iov_len = body_len;
        rc = taida_tls_writev_all(client_fd, iov, 2);
    } else {
        // Bytes body: materialize from taida_val array into contiguous buffer,
        // then send head + body via 2 iovecs. Single materialization, no
        // intermediate encode step.
        unsigned char body_stack[4096];
        unsigned char *body_buf = (body_len <= sizeof(body_stack)) ? body_stack
            : (unsigned char*)TAIDA_MALLOC(body_len, "net_scatter_body");
        for (size_t i = 0; i < body_len; i++) {
            body_buf[i] = (unsigned char)body_bytes_arr[2 + i];
        }
        struct iovec iov[2];
        iov[0].iov_base = head;
        iov[0].iov_len = head_len;
        iov[1].iov_base = body_buf;
        iov[1].iov_len = body_len;
        rc = taida_tls_writev_all(client_fd, iov, 2);
        if (body_buf != body_stack) free(body_buf);
    }

    if (head != head_stack) free(head);
    return rc;
}

// ── NET3-5a/5b/5c/5d/5e: v3 streaming writer state machine ─────────────
// Writer state: Idle(0) → HeadPrepared(1) → Streaming(2) → Ended(3)
// Thread-local context for v3 streaming API. Set in the worker thread
// before invoking a 2-arg handler; the v3 API functions (startResponse,
// writeChunk, endResponse, sseEvent) access it via these thread-locals.

#define NET3_STATE_IDLE         0
#define NET3_STATE_HEAD_PREPARED 1
#define NET3_STATE_STREAMING    2
#define NET3_STATE_ENDED        3
#define NET3_STATE_WEBSOCKET    4

// Maximum pending headers per streaming response
#define NET3_MAX_HEADERS 64

typedef struct {
    int state;               // NET3_STATE_*
    int pending_status;      // default 200
    int sse_mode;            // SSE auto-headers applied
    int header_count;        // number of pending headers
    // Stack-allocated header storage (no per-request malloc for headers)
    const char *header_names[NET3_MAX_HEADERS];
    const char *header_values[NET3_MAX_HEADERS];
} Net3WriterState;

// ── v4 Request Body Streaming State ──────────────────────────
// Per-request state for body-deferred 2-arg handlers.
// Lives on the worker stack; v4 API functions access it via thread-local.

#define NET4_CHUNKED_WAIT_SIZE    0
#define NET4_CHUNKED_READ_DATA    1
#define NET4_CHUNKED_WAIT_TRAILER 2
#define NET4_CHUNKED_DONE         3

typedef struct {
    int is_chunked;          // Transfer-Encoding: chunked?
    int64_t content_length;  // Content-Length from head (0 if absent/chunked)
    int64_t bytes_consumed;  // how many body bytes consumed so far (CL path)
    int fully_read;          // body fully consumed?
    int any_read_started;    // any readBodyChunk/readBodyAll call made?
    // Leftover bytes from head parsing that are body bytes already received.
    unsigned char *leftover;
    size_t leftover_len;
    size_t leftover_pos;     // current position within leftover
    // Chunked decoder state
    int chunked_state;       // NET4_CHUNKED_*
    size_t chunked_remaining;// bytes remaining in current chunk
    // Request-scoped identity token (NB4-7 parity)
    uint64_t request_token;
    // WebSocket close state
    int ws_closed;
    // NB4-10: Connection-scoped WebSocket token for identity verification.
    uint64_t ws_token;
    // v5: Received close code from peer's close frame (0 = not received).
    int64_t ws_close_code;
} Net4BodyState;

// Global monotonic counter for unique request tokens (NB4-7 parity).
static volatile uint64_t taida_net4_next_token = 1;
static uint64_t taida_net4_alloc_token(void) {
    return __atomic_fetch_add(&taida_net4_next_token, 1, __ATOMIC_RELAXED);
}

// NB4-10: Global monotonic counter for unique WebSocket connection tokens.
static volatile uint64_t taida_net4_next_ws_token = 1;
static uint64_t taida_net4_alloc_ws_token(void) {
    return __atomic_fetch_add(&taida_net4_next_ws_token, 1, __ATOMIC_RELAXED);
}

// Thread-local: current writer state and client fd for v3 streaming API.
// These are set/cleared around each 2-arg handler invocation.
static __thread Net3WriterState *tl_net3_writer = NULL;
static __thread int tl_net3_client_fd = -1;
// v4: per-request body streaming state for 2-arg handlers.
static __thread Net4BodyState *tl_net4_body = NULL;

// Forward declaration: writer token validation (defined after create_writer_token).
static void taida_net3_validate_writer(taida_val writer, const char *api_name);

// NET3-5c: writev()-based send helper. Sends all iov buffers, handling
// partial writes and EINTR. Returns 0 on success, -1 on error.
// NET5-4a: Routes through TLS when tl_ssl is active.
static int taida_net_writev_all(int fd, struct iovec *iov, int iovcnt) {
    return taida_tls_writev_all(fd, iov, iovcnt);
}

// Check if a status code forbids a message body (1xx, 204, 205, 304).
static int taida_net3_is_bodyless_status(int status) {
    return (status >= 100 && status <= 199) || status == 204 || status == 205 || status == 304;
}

// Build and send the streaming response head.
// Appends Transfer-Encoding: chunked for non-bodyless status codes.
// Uses stack buffer (no per-request malloc for typical headers).
// Returns 0 on success, -1 on send error, -2 on head overflow.
#define NET3_HEAD_BUF_SIZE 8192
static int taida_net3_commit_head(int fd, Net3WriterState *w) {
    char head_buf[NET3_HEAD_BUF_SIZE];
    size_t cap = sizeof(head_buf);
    size_t offset = 0;
    int n;

    const char *reason = taida_net_status_reason(w->pending_status);
    n = snprintf(head_buf, cap, "HTTP/1.1 %d %s\r\n", w->pending_status, reason);
    if (n < 0 || (size_t)n >= cap) goto overflow;
    offset += (size_t)n;

    for (int i = 0; i < w->header_count && i < NET3_MAX_HEADERS; i++) {
        size_t remaining = cap - offset;
        n = snprintf(head_buf + offset, remaining,
                     "%s: %s\r\n", w->header_names[i], w->header_values[i]);
        if (n < 0 || (size_t)n >= remaining) goto overflow;
        offset += (size_t)n;
    }
    if (!taida_net3_is_bodyless_status(w->pending_status)) {
        size_t remaining = cap - offset;
        n = snprintf(head_buf + offset, remaining, "Transfer-Encoding: chunked\r\n");
        if (n < 0 || (size_t)n >= remaining) goto overflow;
        offset += (size_t)n;
    }
    {
        size_t remaining = cap - offset;
        n = snprintf(head_buf + offset, remaining, "\r\n");
        if (n < 0 || (size_t)n >= remaining) goto overflow;
        offset += (size_t)n;
    }
    return taida_net_send_all(fd, head_buf, offset);

overflow:
    fprintf(stderr, "commit_head: response head exceeds %d bytes (too many or too large headers)\n",
            (int)NET3_HEAD_BUF_SIZE);
    return -2;
}

// Validate reserved headers (Content-Length, Transfer-Encoding) in streaming path.
// Returns 0 if valid, prints error to stderr and returns -1 if invalid.
static int taida_net3_validate_reserved_headers(taida_val headers, const char *api_name) {
    if (!TAIDA_IS_LIST(headers)) return 0;
    taida_val *list = (taida_val*)headers;
    taida_val len = list[2];
    for (taida_val i = 0; i < len; i++) {
        taida_val item = list[4 + i];
        if (!taida_is_buchi_pack(item)) continue;
        taida_val name_val = taida_pack_get(item, taida_str_hash((taida_val)"name"));
        if (name_val == 0) continue;
        const char *name_str = (const char*)name_val;
        size_t name_len = 0;
        if (!taida_read_cstr_len_safe(name_str, 256, &name_len)) continue;
        // Case-insensitive comparison
        if (name_len == 14) {
            // "content-length" (14 chars)
            char lower[15];
            for (size_t j = 0; j < name_len; j++) lower[j] = (char)((name_str[j] >= 'A' && name_str[j] <= 'Z') ? name_str[j] + 32 : name_str[j]);
            lower[name_len] = '\0';
            if (strcmp(lower, "content-length") == 0) {
                fprintf(stderr, "%s: 'Content-Length' is not allowed in streaming response headers. "
                        "The runtime manages Content-Length/Transfer-Encoding for streaming responses.\n", api_name);
                return -1;
            }
        }
        if (name_len == 17) {
            // "transfer-encoding" (17 chars)
            char lower[18];
            for (size_t j = 0; j < name_len; j++) lower[j] = (char)((name_str[j] >= 'A' && name_str[j] <= 'Z') ? name_str[j] + 32 : name_str[j]);
            lower[name_len] = '\0';
            if (strcmp(lower, "transfer-encoding") == 0) {
                fprintf(stderr, "%s: 'Transfer-Encoding' is not allowed in streaming response headers. "
                        "The runtime manages Transfer-Encoding for streaming responses.\n", api_name);
                return -1;
            }
        }
    }
    return 0;
}

// Extract headers from a taida list of @(name, value) packs into the writer state.
static void taida_net3_extract_headers(Net3WriterState *w, taida_val headers) {
    w->header_count = 0;
    if (!TAIDA_IS_LIST(headers)) return;
    taida_val *list = (taida_val*)headers;
    taida_val len = list[2];
    for (taida_val i = 0; i < len && w->header_count < NET3_MAX_HEADERS; i++) {
        taida_val item = list[4 + i];
        if (!taida_is_buchi_pack(item)) continue;
        taida_val name_val = taida_pack_get(item, taida_str_hash((taida_val)"name"));
        taida_val value_val = taida_pack_get(item, taida_str_hash((taida_val)"value"));
        if (name_val == 0 || value_val == 0) continue;
        w->header_names[w->header_count] = (const char*)name_val;
        w->header_values[w->header_count] = (const char*)value_val;
        w->header_count++;
    }
}

// NET3-5b: startResponse(writer, status, headers)
// Updates pending status/headers on the writer state. Does NOT commit to wire.
taida_val taida_net_start_response(taida_val writer, taida_val status, taida_val headers) {
    taida_net3_validate_writer(writer, "startResponse");
    Net3WriterState *w = tl_net3_writer;
    if (!w) {
        fprintf(stderr, "startResponse: can only be called inside a 2-argument httpServe handler\n");
        exit(1);
    }
    // State check
    switch (w->state) {
        case NET3_STATE_IDLE: break;
        case NET3_STATE_HEAD_PREPARED:
            fprintf(stderr, "startResponse: already called. Cannot call startResponse twice.\n");
            exit(1);
        case NET3_STATE_STREAMING:
            fprintf(stderr, "startResponse: head already committed (chunks are being written). Cannot change status/headers after writeChunk.\n");
            exit(1);
        case NET3_STATE_ENDED:
            fprintf(stderr, "startResponse: response already ended.\n");
            exit(1);
    }
    // Validate status range
    if (status < 100 || status > 599) {
        fprintf(stderr, "startResponse: status must be 100-599, got %lld\n", (long long)status);
        exit(1);
    }
    // Validate reserved headers
    if (taida_net3_validate_reserved_headers(headers, "startResponse") < 0) {
        exit(1);
    }
    w->pending_status = (int)status;
    taida_net3_extract_headers(w, headers);
    w->state = NET3_STATE_HEAD_PREPARED;
    return 0; // Unit
}

// NET3-5b/5c/5d: writeChunk(writer, data)
// Sends one chunk of body data using chunked TE. Uses writev() for zero-copy.
// Bytes: extract from taida_val array to stack/stack-heap buffer, then writev.
// Str: use C string directly.
taida_val taida_net_write_chunk(taida_val writer, taida_val data) {
    taida_net3_validate_writer(writer, "writeChunk");
    Net3WriterState *w = tl_net3_writer;
    int fd = tl_net3_client_fd;
    if (!w) {
        fprintf(stderr, "writeChunk: can only be called inside a 2-argument httpServe handler\n");
        exit(1);
    }
    if (w->state == NET3_STATE_ENDED) {
        fprintf(stderr, "writeChunk: response already ended.\n");
        exit(1);
    }

    // Extract payload pointer and length
    const unsigned char *payload = NULL;
    size_t payload_len = 0;
    // NET3-5d: For Bytes, we need to convert from taida_val array to contiguous bytes.
    // Use stack buffer for small payloads, heap only for large ones. No per-chunk persistent alloc.
    unsigned char stack_payload[4096];
    unsigned char *heap_payload = NULL;
    int is_bytes = 0;

    if (TAIDA_IS_BYTES(data)) {
        is_bytes = 1;
        taida_val *bytes = (taida_val*)data;
        taida_val blen = bytes[1];
        payload_len = (size_t)blen;
        if (payload_len == 0) return 0; // empty chunk is no-op
        if (payload_len <= sizeof(stack_payload)) {
            for (size_t i = 0; i < payload_len; i++) stack_payload[i] = (unsigned char)bytes[2 + i];
            payload = stack_payload;
        } else {
            heap_payload = (unsigned char*)TAIDA_MALLOC(payload_len, "net3_write_chunk_bytes");
            for (size_t i = 0; i < payload_len; i++) heap_payload[i] = (unsigned char)bytes[2 + i];
            payload = heap_payload;
        }
    } else {
        // Assume Str (C string)
        const char *str = (const char*)data;
        size_t slen = 0;
        if (!taida_read_cstr_len_safe(str, 16 * 1024 * 1024, &slen)) {
            fprintf(stderr, "writeChunk: data must be Bytes or Str\n");
            if (heap_payload) free(heap_payload);
            exit(1);
        }
        payload = (const unsigned char*)str;
        payload_len = slen;
        if (payload_len == 0) return 0; // empty chunk is no-op
    }

    // Bodyless status check
    if (taida_net3_is_bodyless_status(w->pending_status)) {
        fprintf(stderr, "writeChunk: status %d does not allow a message body\n", w->pending_status);
        if (heap_payload) free(heap_payload);
        exit(1);
    }

    // Commit head if not yet committed
    if (w->state == NET3_STATE_IDLE || w->state == NET3_STATE_HEAD_PREPARED) {
        if (taida_net3_commit_head(fd, w) != 0) {
            fprintf(stderr, "writeChunk: failed to commit response head\n");
            if (heap_payload) free(heap_payload);
            exit(1);
        }
        w->state = NET3_STATE_STREAMING;
    }

    // NET3-5c: Send chunk using writev() — zero-copy for payload.
    // Wire format: <hex-size>\r\n<payload>\r\n
    char hex_prefix[32];
    int hex_len = snprintf(hex_prefix, sizeof(hex_prefix), "%zx\r\n", payload_len);

    struct iovec iov[3];
    iov[0].iov_base = hex_prefix;
    iov[0].iov_len = (size_t)hex_len;
    iov[1].iov_base = (void*)payload;
    iov[1].iov_len = payload_len;
    iov[2].iov_base = (void*)"\r\n";
    iov[2].iov_len = 2;

    // NB3-5: Check writev_all return value for write errors (e.g. peer RST).
    if (taida_net_writev_all(fd, iov, 3) != 0) {
        if (heap_payload) free(heap_payload);
        fprintf(stderr, "writeChunk: failed to send chunk data\n");
        exit(1);
    }

    if (heap_payload) free(heap_payload);
    return 0; // Unit
}

// NET3-5b: endResponse(writer)
// Terminates the chunked response by sending 0\r\n\r\n.
// Idempotent: second call is a no-op.
taida_val taida_net_end_response(taida_val writer) {
    taida_net3_validate_writer(writer, "endResponse");
    Net3WriterState *w = tl_net3_writer;
    int fd = tl_net3_client_fd;
    if (!w) {
        fprintf(stderr, "endResponse: can only be called inside a 2-argument httpServe handler\n");
        exit(1);
    }
    // Idempotent: no-op if already ended
    if (w->state == NET3_STATE_ENDED) return 0;

    // Commit head if not yet committed
    if (w->state == NET3_STATE_IDLE || w->state == NET3_STATE_HEAD_PREPARED) {
        if (taida_net3_commit_head(fd, w) != 0) {
            fprintf(stderr, "endResponse: failed to commit response head\n");
            exit(1);
        }
    }

    // Send chunked terminator — but only for non-bodyless status
    if (!taida_net3_is_bodyless_status(w->pending_status)) {
        taida_net_send_all(fd, "0\r\n\r\n", 5);
    }
    w->state = NET3_STATE_ENDED;
    return 0; // Unit
}

// NET3-5e: sseEvent(writer, event, data)
// SSE convenience API. Sends one Server-Sent Event.
// Auto-sets Content-Type and Cache-Control headers if not already set.
// Splits multiline data into data: lines.
taida_val taida_net_sse_event(taida_val writer, taida_val event, taida_val data) {
    taida_net3_validate_writer(writer, "sseEvent");
    Net3WriterState *w = tl_net3_writer;
    int fd = tl_net3_client_fd;
    if (!w) {
        fprintf(stderr, "sseEvent: can only be called inside a 2-argument httpServe handler\n");
        exit(1);
    }
    // Validate event and data are strings.
    // NB3-8: Use taida_str_byte_len which reads heap string length from header
    // metadata instead of scanning for NUL. This is correct for non-ASCII
    // (multi-byte UTF-8) strings and avoids parity issues with Interpreter/JS.
    const char *event_str = (const char*)event;
    const char *data_str = (const char*)data;
    size_t event_len = 0, data_len = 0;
    if (!taida_str_byte_len(event_str, &event_len)) {
        fprintf(stderr, "sseEvent: event must be Str\n");
        exit(1);
    }
    if (!taida_str_byte_len(data_str, &data_len)) {
        fprintf(stderr, "sseEvent: data must be Str\n");
        exit(1);
    }

    if (w->state == NET3_STATE_ENDED) {
        fprintf(stderr, "sseEvent: response already ended.\n");
        exit(1);
    }
    if (taida_net3_is_bodyless_status(w->pending_status)) {
        fprintf(stderr, "sseEvent: status %d does not allow a message body\n", w->pending_status);
        exit(1);
    }

    // SSE auto-headers (once per writer)
    if (!w->sse_mode) {
        if (w->state == NET3_STATE_STREAMING) {
            // Head already committed — check if SSE headers were set
            int has_ct = 0, has_cc = 0;
            for (int i = 0; i < w->header_count; i++) {
                const char *n = w->header_names[i];
                size_t nlen = 0;
                if (!taida_read_cstr_len_safe(n, 256, &nlen)) continue;
                // Case-insensitive check
                if (nlen == 12) {
                    char lower[13];
                    for (size_t j = 0; j < nlen; j++) lower[j] = (char)((n[j] >= 'A' && n[j] <= 'Z') ? n[j] + 32 : n[j]);
                    lower[nlen] = '\0';
                    if (strcmp(lower, "content-type") == 0) {
                        const char *v = w->header_values[i];
                        size_t vlen = 0;
                        if (taida_read_cstr_len_safe(v, 256, &vlen)) {
                            char lv[256];
                            for (size_t j = 0; j < vlen && j < 255; j++) lv[j] = (char)((v[j] >= 'A' && v[j] <= 'Z') ? v[j] + 32 : v[j]);
                            lv[vlen < 255 ? vlen : 255] = '\0';
                            if (strstr(lv, "text/event-stream")) has_ct = 1;
                        }
                    }
                }
                if (nlen == 13) {
                    char lower[14];
                    for (size_t j = 0; j < nlen; j++) lower[j] = (char)((n[j] >= 'A' && n[j] <= 'Z') ? n[j] + 32 : n[j]);
                    lower[nlen] = '\0';
                    if (strcmp(lower, "cache-control") == 0) {
                        const char *v = w->header_values[i];
                        size_t vlen = 0;
                        if (taida_read_cstr_len_safe(v, 256, &vlen)) {
                            char lv[256];
                            for (size_t j = 0; j < vlen && j < 255; j++) lv[j] = (char)((v[j] >= 'A' && v[j] <= 'Z') ? v[j] + 32 : v[j]);
                            lv[vlen < 255 ? vlen : 255] = '\0';
                            if (strstr(lv, "no-cache")) has_cc = 1;
                        }
                    }
                }
            }
            if (!has_ct || !has_cc) {
                fprintf(stderr, "sseEvent: head already committed without SSE headers. "
                        "Call sseEvent before writeChunk, or use startResponse "
                        "with explicit Content-Type: text/event-stream and "
                        "Cache-Control: no-cache headers before writeChunk.\n");
                exit(1);
            }
            w->sse_mode = 1;
        } else {
            // Head not yet committed — safe to add auto-headers
            int has_ct = 0, has_cc = 0;
            for (int i = 0; i < w->header_count; i++) {
                const char *n = w->header_names[i];
                size_t nlen = 0;
                if (!taida_read_cstr_len_safe(n, 256, &nlen)) continue;
                char lower[256];
                for (size_t j = 0; j < nlen && j < 255; j++) lower[j] = (char)((n[j] >= 'A' && n[j] <= 'Z') ? n[j] + 32 : n[j]);
                lower[nlen < 255 ? nlen : 255] = '\0';
                if (strcmp(lower, "content-type") == 0) has_ct = 1;
                if (strcmp(lower, "cache-control") == 0) has_cc = 1;
            }
            if (!has_ct && w->header_count < NET3_MAX_HEADERS) {
                w->header_names[w->header_count] = "Content-Type";
                w->header_values[w->header_count] = "text/event-stream; charset=utf-8";
                w->header_count++;
            }
            if (!has_cc && w->header_count < NET3_MAX_HEADERS) {
                w->header_names[w->header_count] = "Cache-Control";
                w->header_values[w->header_count] = "no-cache";
                w->header_count++;
            }
            w->sse_mode = 1;
        }
    }

    // Commit head if not yet committed
    if (w->state == NET3_STATE_IDLE || w->state == NET3_STATE_HEAD_PREPARED) {
        if (taida_net3_commit_head(fd, w) != 0) {
            fprintf(stderr, "sseEvent: failed to commit response head\n");
            exit(1);
        }
        w->state = NET3_STATE_STREAMING;
    }

    // Build SSE event payload and compute total length.
    // Wire format:
    //   event: <event>\n      (omit if event is empty)
    //   data: <line1>\n
    //   data: <line2>\n       (for each line in data split by \n)
    //   \n                    (event terminator)

    // Count data lines
    int line_count = 1;
    for (size_t i = 0; i < data_len; i++) {
        if (data_str[i] == '\n') line_count++;
    }

    // Compute total payload length for chunk header
    size_t total_payload = 0;
    if (event_len > 0) {
        total_payload += 7 + event_len + 1; // "event: " + event + "\n"
    }
    // For each data line: "data: " + line + "\n"
    {
        const char *p = data_str;
        const char *end = data_str + data_len;
        while (p <= end) {
            const char *nl = p;
            while (nl < end && *nl != '\n') nl++;
            size_t line_len = (size_t)(nl - p);
            total_payload += 6 + line_len + 1; // "data: " + line + "\n"
            p = nl + 1;
            if (nl == end) break;
        }
    }
    total_payload += 1; // terminator "\n"

    // Build chunk: hex_prefix + SSE payload + chunk suffix
    char hex_prefix[32];
    int hex_len = snprintf(hex_prefix, sizeof(hex_prefix), "%zx\r\n", total_payload);

    // Use iov array. Max iovecs: 1(hex) + 3(event line) + 3*line_count(data lines) + 1(term) + 1(suffix)
    int max_iov = 1 + 3 + 3 * line_count + 1 + 1;
    // Use stack for small SSE events, heap for large
    struct iovec stack_iov[64];
    struct iovec *iov = (max_iov <= 64) ? stack_iov : (struct iovec*)TAIDA_MALLOC(sizeof(struct iovec) * (size_t)max_iov, "net3_sse_iov");
    int iov_count = 0;

    // hex prefix
    iov[iov_count].iov_base = hex_prefix;
    iov[iov_count].iov_len = (size_t)hex_len;
    iov_count++;

    // event: line
    if (event_len > 0) {
        iov[iov_count].iov_base = (void*)"event: ";
        iov[iov_count].iov_len = 7;
        iov_count++;
        iov[iov_count].iov_base = (void*)event_str;
        iov[iov_count].iov_len = event_len;
        iov_count++;
        iov[iov_count].iov_base = (void*)"\n";
        iov[iov_count].iov_len = 1;
        iov_count++;
    }

    // data: lines
    {
        const char *p = data_str;
        const char *end = data_str + data_len;
        while (p <= end) {
            const char *nl = p;
            while (nl < end && *nl != '\n') nl++;
            size_t line_len = (size_t)(nl - p);
            iov[iov_count].iov_base = (void*)"data: ";
            iov[iov_count].iov_len = 6;
            iov_count++;
            if (line_len > 0) {
                iov[iov_count].iov_base = (void*)p;
                iov[iov_count].iov_len = line_len;
                iov_count++;
            }
            iov[iov_count].iov_base = (void*)"\n";
            iov[iov_count].iov_len = 1;
            iov_count++;
            p = nl + 1;
            if (nl == end) break;
        }
    }

    // event terminator
    iov[iov_count].iov_base = (void*)"\n";
    iov[iov_count].iov_len = 1;
    iov_count++;

    // chunk suffix
    iov[iov_count].iov_base = (void*)"\r\n";
    iov[iov_count].iov_len = 2;
    iov_count++;

    // NB3-5: Check writev_all return value for write errors (e.g. peer RST).
    if (taida_net_writev_all(fd, iov, iov_count) != 0) {
        if (iov != stack_iov) free(iov);
        fprintf(stderr, "sseEvent: failed to send SSE chunk data\n");
        exit(1);
    }

    if (iov != stack_iov) free(iov);

    return 0; // Unit
}

// ── NET4-4: v4 Request Body Streaming + WebSocket — Native backend ──
//
// Phase 4: Full implementation of readBodyChunk, readBodyAll,
// wsUpgrade, wsSend, wsReceive, wsClose.
// Replaces NB4-6 stubs.

// ── SHA-1 implementation (RFC 3174, ~100 lines) ─────────────
// Used exclusively for WebSocket Sec-WebSocket-Accept calculation.
// Not for cryptographic purposes.

static void taida_sha1_transform(uint32_t state[5], const uint8_t block[64]) {
    uint32_t w[80];
    for (int i = 0; i < 16; i++) {
        w[i] = ((uint32_t)block[i*4] << 24) | ((uint32_t)block[i*4+1] << 16)
             | ((uint32_t)block[i*4+2] << 8) | (uint32_t)block[i*4+3];
    }
    for (int i = 16; i < 80; i++) {
        uint32_t t = w[i-3] ^ w[i-8] ^ w[i-14] ^ w[i-16];
        w[i] = (t << 1) | (t >> 31);
    }
    uint32_t a = state[0], b = state[1], c = state[2], d = state[3], e = state[4];
    for (int i = 0; i < 80; i++) {
        uint32_t f, k;
        if (i < 20)      { f = (b & c) | ((~b) & d); k = 0x5A827999; }
        else if (i < 40) { f = b ^ c ^ d;             k = 0x6ED9EBA1; }
        else if (i < 60) { f = (b & c) | (b & d) | (c & d); k = 0x8F1BBCDC; }
        else              { f = b ^ c ^ d;             k = 0xCA62C1D6; }
        uint32_t temp = ((a << 5) | (a >> 27)) + f + e + k + w[i];
        e = d; d = c; c = (b << 30) | (b >> 2); b = a; a = temp;
    }
    state[0] += a; state[1] += b; state[2] += c; state[3] += d; state[4] += e;
}

// SHA-1 hash: input -> 20-byte digest.
static void taida_sha1(const uint8_t *data, size_t len, uint8_t digest[20]) {
    uint32_t state[5] = { 0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0 };
    size_t i;
    uint8_t block[64];
    size_t block_pos = 0;

    for (i = 0; i < len; i++) {
        block[block_pos++] = data[i];
        if (block_pos == 64) {
            taida_sha1_transform(state, block);
            block_pos = 0;
        }
    }

    // Padding
    block[block_pos++] = 0x80;
    if (block_pos > 56) {
        while (block_pos < 64) block[block_pos++] = 0;
        taida_sha1_transform(state, block);
        block_pos = 0;
    }
    while (block_pos < 56) block[block_pos++] = 0;

    // Length in bits (big-endian 64-bit)
    uint64_t bit_len = (uint64_t)len * 8;
    block[56] = (uint8_t)(bit_len >> 56);
    block[57] = (uint8_t)(bit_len >> 48);
    block[58] = (uint8_t)(bit_len >> 40);
    block[59] = (uint8_t)(bit_len >> 32);
    block[60] = (uint8_t)(bit_len >> 24);
    block[61] = (uint8_t)(bit_len >> 16);
    block[62] = (uint8_t)(bit_len >> 8);
    block[63] = (uint8_t)(bit_len);
    taida_sha1_transform(state, block);

    for (i = 0; i < 5; i++) {
        digest[i*4]   = (uint8_t)(state[i] >> 24);
        digest[i*4+1] = (uint8_t)(state[i] >> 16);
        digest[i*4+2] = (uint8_t)(state[i] >> 8);
        digest[i*4+3] = (uint8_t)(state[i]);
    }
}

// ── Base64 encode ──────────────────────────────────────────
static const char taida_b64_chars[] =
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

// Base64 encode: input bytes -> null-terminated string (caller must free).
static char *taida_base64_encode(const uint8_t *data, size_t len) {
    size_t out_len = 4 * ((len + 2) / 3);
    char *out = (char*)TAIDA_MALLOC(out_len + 1, "net_base64");
    size_t j = 0;
    for (size_t i = 0; i < len; ) {
        uint32_t octet_a = i < len ? data[i++] : 0;
        uint32_t octet_b = i < len ? data[i++] : 0;
        uint32_t octet_c = i < len ? data[i++] : 0;
        uint32_t triple = (octet_a << 16) | (octet_b << 8) | octet_c;
        out[j++] = taida_b64_chars[(triple >> 18) & 0x3F];
        out[j++] = taida_b64_chars[(triple >> 12) & 0x3F];
        out[j++] = taida_b64_chars[(triple >> 6) & 0x3F];
        out[j++] = taida_b64_chars[triple & 0x3F];
    }
    // Padding
    size_t mod = len % 3;
    if (mod == 1) { out[j-1] = '='; out[j-2] = '='; }
    else if (mod == 2) { out[j-1] = '='; }
    out[j] = '\0';
    return out;
}

// NB4-11: Base64 decode for Sec-WebSocket-Key validation.
// Returns decoded length, or -1 on invalid input. Writes to `out` (must have enough space).
static int taida_base64_decode(const char *input, size_t input_len, uint8_t *out, size_t out_cap) {
    static const int8_t decode_table[256] = {
        [0 ... 255] = -1,
        ['A'] = 0, ['B'] = 1, ['C'] = 2, ['D'] = 3, ['E'] = 4, ['F'] = 5,
        ['G'] = 6, ['H'] = 7, ['I'] = 8, ['J'] = 9, ['K'] = 10, ['L'] = 11,
        ['M'] = 12, ['N'] = 13, ['O'] = 14, ['P'] = 15, ['Q'] = 16, ['R'] = 17,
        ['S'] = 18, ['T'] = 19, ['U'] = 20, ['V'] = 21, ['W'] = 22, ['X'] = 23,
        ['Y'] = 24, ['Z'] = 25,
        ['a'] = 26, ['b'] = 27, ['c'] = 28, ['d'] = 29, ['e'] = 30, ['f'] = 31,
        ['g'] = 32, ['h'] = 33, ['i'] = 34, ['j'] = 35, ['k'] = 36, ['l'] = 37,
        ['m'] = 38, ['n'] = 39, ['o'] = 40, ['p'] = 41, ['q'] = 42, ['r'] = 43,
        ['s'] = 44, ['t'] = 45, ['u'] = 46, ['v'] = 47, ['w'] = 48, ['x'] = 49,
        ['y'] = 50, ['z'] = 51,
        ['0'] = 52, ['1'] = 53, ['2'] = 54, ['3'] = 55, ['4'] = 56, ['5'] = 57,
        ['6'] = 58, ['7'] = 59, ['8'] = 60, ['9'] = 61,
        ['+'] = 62, ['/'] = 63
    };
    if (input_len % 4 != 0) return -1;
    size_t decoded_len = input_len / 4 * 3;
    if (input_len > 0 && input[input_len - 1] == '=') decoded_len--;
    if (input_len > 1 && input[input_len - 2] == '=') decoded_len--;
    if (decoded_len > out_cap) return -1;

    size_t j = 0;
    for (size_t i = 0; i < input_len; i += 4) {
        int8_t a = decode_table[(unsigned char)input[i]];
        int8_t b = (i + 1 < input_len) ? decode_table[(unsigned char)input[i + 1]] : -1;
        if (a < 0 || b < 0) return -1;
        uint32_t triple = ((uint32_t)a << 18) | ((uint32_t)b << 12);
        if (i + 2 < input_len && input[i + 2] != '=') {
            int8_t c = decode_table[(unsigned char)input[i + 2]];
            if (c < 0) return -1;
            triple |= ((uint32_t)c << 6);
        }
        if (i + 3 < input_len && input[i + 3] != '=') {
            int8_t d = decode_table[(unsigned char)input[i + 3]];
            if (d < 0) return -1;
            triple |= (uint32_t)d;
        }
        if (j < decoded_len) out[j++] = (uint8_t)(triple >> 16);
        if (j < decoded_len) out[j++] = (uint8_t)(triple >> 8);
        if (j < decoded_len) out[j++] = (uint8_t)triple;
    }
    return (int)decoded_len;
}

// ── Compute Sec-WebSocket-Accept (NET4-4b) ──────────────────
// SHA-1(key + GUID) -> Base64
static const char *WS_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

static char *taida_net4_compute_ws_accept(const char *key) {
    // Concatenate key + GUID
    size_t key_len = strlen(key);
    size_t guid_len = strlen(WS_GUID);
    size_t total = key_len + guid_len;
    uint8_t *combined = (uint8_t*)TAIDA_MALLOC(total + 1, "net_ws_accept");
    memcpy(combined, key, key_len);
    memcpy(combined + key_len, WS_GUID, guid_len);

    uint8_t digest[20];
    taida_sha1(combined, total, digest);
    free(combined);

    return taida_base64_encode(digest, 20);
}

// ── WebSocket constants ──────────────────────────────────────
#define WS_OPCODE_TEXT   0x1
#define WS_OPCODE_BINARY 0x2
#define WS_OPCODE_CLOSE  0x8
#define WS_OPCODE_PING   0x9
#define WS_OPCODE_PONG   0xA
#define WS_MAX_PAYLOAD   (16ULL * 1024 * 1024)  // 16 MiB

// ── v4 Body streaming helpers ────────────────────────────────

// Read exactly `count` bytes from fd. Returns bytes read, 0 on error/EOF.
// NET5-4a: Routes through TLS when tl_ssl is active.
static size_t taida_net4_recv_exact(int fd, unsigned char *out, size_t count) {
    return taida_tls_recv_exact(fd, out, count);
}

// Read up to `count` bytes from leftover then fd.
// Returns a new Bytes object (caller's ownership), or empty Bytes on EOF.
// NET5-4a: Routes through TLS when tl_ssl is active.
static size_t taida_net4_read_body_bytes(Net4BodyState *bs, int fd, unsigned char *out, size_t count) {
    size_t total = 0;
    // First, drain from leftover.
    while (total < count && bs->leftover_pos < bs->leftover_len) {
        out[total++] = bs->leftover[bs->leftover_pos++];
    }
    // Then read from socket (TLS-aware).
    while (total < count) {
        ssize_t n = taida_tls_recv(fd, out + total, count - total);
        if (n <= 0) {
            if (n < 0 && errno == EINTR) continue;
            break; // EOF or error
        }
        total += (size_t)n;
    }
    return total;
}

// Read a line (up to LF) from leftover then fd.
// Returns line in `out` (null-terminated). Max `cap` bytes including NUL.
// Returns length excluding NUL.
// NET5-4a: Routes through TLS when tl_ssl is active.
static size_t taida_net4_read_line(Net4BodyState *bs, int fd, char *out, size_t cap) {
    size_t pos = 0;
    // From leftover.
    while (pos < cap - 1 && bs->leftover_pos < bs->leftover_len) {
        unsigned char b = bs->leftover[bs->leftover_pos++];
        out[pos++] = (char)b;
        if (b == '\n') { out[pos] = '\0'; return pos; }
    }
    // From socket byte-by-byte (TLS-aware).
    while (pos < cap - 1) {
        unsigned char b;
        ssize_t n = taida_tls_recv(fd, &b, 1);
        if (n <= 0) {
            if (n < 0 && errno == EINTR) continue;
            break;
        }
        out[pos++] = (char)b;
        if (b == '\n') break;
    }
    out[pos] = '\0';
    return pos;
}

// Drain chunked trailers after terminal chunk (NB4-8 parity).
// Returns 0 on success, -1 on protocol error (missing final CRLF).
static int taida_net4_drain_chunked_trailers(Net4BodyState *bs, int fd) {
    char line[4096];
    for (int i = 0; i < 64; i++) {
        size_t len = taida_net4_read_line(bs, fd, line, sizeof(line));
        // NB4-18: EOF (0 raw bytes) != valid empty line ("\r\n").
        if (len == 0) {
            fprintf(stderr, "chunked body error: missing final CRLF after terminal chunk\n");
            return -1;
        }
        // Trim whitespace and check empty.
        size_t start = 0, end = len;
        while (start < end && (line[start] == ' ' || line[start] == '\t' || line[start] == '\r' || line[start] == '\n')) start++;
        while (end > start && (line[end-1] == ' ' || line[end-1] == '\t' || line[end-1] == '\r' || line[end-1] == '\n')) end--;
        if (start == end) return 0; // Empty line = trailers done.
    }
    return 0;
}

// Make Lax[Bytes] empty (parity with Interpreter: hasValue=false).
static taida_val taida_net4_make_lax_bytes_empty(void) {
    return taida_lax_empty(taida_bytes_default_value());
}

// Make Lax[Bytes] with value (parity with Interpreter: hasValue=true).
static taida_val taida_net4_make_lax_bytes_value(const unsigned char *data, size_t len) {
    taida_val bytes = taida_bytes_from_raw(data, (taida_val)len);
    return taida_lax_new(bytes, taida_bytes_default_value());
}

// Validate that req is a body-streaming request pack.
static int taida_net4_is_body_stream_request(taida_val req) {
    if (!taida_is_buchi_pack(req)) return 0;
    taida_val sentinel = taida_pack_get(req, taida_str_hash((taida_val)"__body_stream"));
    if (sentinel == 0) return 0;
    const char *s = (const char*)sentinel;
    size_t slen = 0;
    if (!taida_read_cstr_len_safe(s, 64, &slen)) return 0;
    return (slen == 16 && memcmp(s, "__v4_body_stream", 16) == 0);
}

// Extract __body_token from request pack.
static uint64_t taida_net4_extract_body_token(taida_val req) {
    return (uint64_t)taida_pack_get(req, taida_str_hash((taida_val)"__body_token"));
}

// ── readBodyChunk(req) → Lax[Bytes] ─────────────────────────
taida_val taida_net_read_body_chunk(taida_val req) {
    if (!taida_net4_is_body_stream_request(req)) {
        fprintf(stderr, "readBodyChunk: can only be called in a 2-argument httpServe handler. "
                "In a 1-argument handler, the request body is already fully read. "
                "Use readBody(req) instead.\n");
        exit(1);
    }

    Net4BodyState *bs = tl_net4_body;
    if (!bs) {
        fprintf(stderr, "readBodyChunk: no active body streaming state\n");
        exit(1);
    }

    // NB4-7: Verify request token.
    uint64_t tok = taida_net4_extract_body_token(req);
    if (tok != bs->request_token) {
        fprintf(stderr, "readBodyChunk: request pack does not match the current active request. "
                "The request may be stale or fabricated.\n");
        exit(1);
    }

    Net3WriterState *w = tl_net3_writer;
    if (w && w->state == NET3_STATE_WEBSOCKET) {
        fprintf(stderr, "readBodyChunk: cannot read HTTP body after WebSocket upgrade.\n");
        exit(1);
    }

    int fd = tl_net3_client_fd;

    bs->any_read_started = 1;

    if (bs->fully_read) {
        return taida_net4_make_lax_bytes_empty();
    }

    if (bs->is_chunked) {
        // Chunked TE decode (parity with Interpreter).
        #define NET4_READ_BUF 8192
        char line_buf[4096];
        for (;;) {
            switch (bs->chunked_state) {
                case NET4_CHUNKED_DONE:
                    bs->fully_read = 1;
                    return taida_net4_make_lax_bytes_empty();

                case NET4_CHUNKED_WAIT_SIZE: {
                    size_t llen = taida_net4_read_line(bs, fd, line_buf, sizeof(line_buf));
                    // Trim.
                    size_t s = 0, e = llen;
                    while (s < e && (line_buf[s]==' '||line_buf[s]=='\t'||line_buf[s]=='\r'||line_buf[s]=='\n')) s++;
                    while (e > s && (line_buf[e-1]==' '||line_buf[e-1]=='\t'||line_buf[e-1]=='\r'||line_buf[e-1]=='\n')) e--;
                    if (s == e) continue; // Empty line, try again.
                    // Parse hex chunk-size (strip chunk-extension after ';').
                    char hex_buf[64];
                    size_t hex_len = 0;
                    for (size_t i = s; i < e && line_buf[i] != ';' && hex_len < 63; i++) {
                        if (line_buf[i] != ' ' && line_buf[i] != '\t')
                            hex_buf[hex_len++] = line_buf[i];
                    }
                    hex_buf[hex_len] = '\0';
                    // NB4-18: Strict hex-only parse. Reject partial parse like '1g'.
                    for (size_t vi = 0; vi < hex_len; vi++) {
                        char c = hex_buf[vi];
                        if (!((c >= '0' && c <= '9') || (c >= 'a' && c <= 'f') || (c >= 'A' && c <= 'F'))) {
                            fprintf(stderr, "readBodyChunk: invalid chunk-size '%s' in chunked body\n", hex_buf);
                            exit(1);
                        }
                    }
                    if (hex_len == 0) continue; // skip empty, retry
                    unsigned long chunk_size = strtoul(hex_buf, NULL, 16);
                    if (chunk_size == 0) {
                        bs->chunked_state = NET4_CHUNKED_DONE;
                        bs->fully_read = 1;
                        if (taida_net4_drain_chunked_trailers(bs, fd) < 0) {
                            bs->fully_read = 0;
                            fprintf(stderr, "readBodyChunk: chunked body protocol error\n");
                            exit(1);
                        }
                        return taida_net4_make_lax_bytes_empty();
                    }
                    bs->chunked_state = NET4_CHUNKED_READ_DATA;
                    bs->chunked_remaining = (size_t)chunk_size;
                    break;
                }

                case NET4_CHUNKED_READ_DATA: {
                    if (bs->chunked_remaining == 0) {
                        bs->chunked_state = NET4_CHUNKED_WAIT_TRAILER;
                        continue;
                    }
                    size_t to_read = bs->chunked_remaining;
                    if (to_read > NET4_READ_BUF) to_read = NET4_READ_BUF;
                    unsigned char tmp[NET4_READ_BUF];
                    size_t got = taida_net4_read_body_bytes(bs, fd, tmp, to_read);
                    // NB4-18: short read (EOF) in chunked data is a protocol error.
                    if (got == 0) {
                        fprintf(stderr, "readBodyChunk: truncated chunked body — expected %zu more chunk-data bytes but got EOF\n",
                                bs->chunked_remaining);
                        exit(1);
                    }
                    bs->chunked_remaining -= got;
                    bs->bytes_consumed += (int64_t)got;
                    return taida_net4_make_lax_bytes_value(tmp, got);
                }

                case NET4_CHUNKED_WAIT_TRAILER: {
                    // NB4-18: Read CRLF after chunk data and validate.
                    {
                        size_t tl_len = taida_net4_read_line(bs, fd, line_buf, sizeof(line_buf));
                        if (tl_len == 0) {
                            fprintf(stderr, "readBodyChunk: missing CRLF after chunk data (unexpected EOF)\n");
                            exit(1);
                        }
                        // Trim and check empty.
                        size_t ts = 0, te = tl_len;
                        while (ts < te && (line_buf[ts]==' '||line_buf[ts]=='\t'||line_buf[ts]=='\r'||line_buf[ts]=='\n')) ts++;
                        while (te > ts && (line_buf[te-1]==' '||line_buf[te-1]=='\t'||line_buf[te-1]=='\r'||line_buf[te-1]=='\n')) te--;
                        if (ts != te) {
                            line_buf[tl_len < sizeof(line_buf)-1 ? tl_len : sizeof(line_buf)-1] = '\0';
                            fprintf(stderr, "readBodyChunk: malformed chunk trailer — expected CRLF after chunk data, got \"%s\"\n", line_buf);
                            exit(1);
                        }
                    }
                    bs->chunked_state = NET4_CHUNKED_WAIT_SIZE;
                    break;
                }
            }
        }
        #undef NET4_READ_BUF
    } else {
        // Content-Length path.
        int64_t remaining = bs->content_length - bs->bytes_consumed;
        if (remaining <= 0) {
            bs->fully_read = 1;
            return taida_net4_make_lax_bytes_empty();
        }
        size_t to_read = (size_t)remaining;
        if (to_read > 8192) to_read = 8192;
        unsigned char tmp[8192];
        size_t got = taida_net4_read_body_bytes(bs, fd, tmp, to_read);
        if (got == 0) {
            // NB4-18: EOF before Content-Length exhausted is a protocol error.
            fprintf(stderr, "readBodyChunk: truncated body — expected %" PRId64
                    " bytes (Content-Length) but got EOF after %" PRId64 " bytes\n",
                    bs->content_length, bs->bytes_consumed);
            exit(1);
        }
        bs->bytes_consumed += (int64_t)got;
        if (bs->bytes_consumed >= bs->content_length) {
            bs->fully_read = 1;
        }
        return taida_net4_make_lax_bytes_value(tmp, got);
    }
}

// ── readBodyAll(req) → Bytes ─────────────────────────────────
// The only aggregate path permitted by v4 contract.
taida_val taida_net_read_body_all(taida_val req) {
    if (!taida_net4_is_body_stream_request(req)) {
        fprintf(stderr, "readBodyAll: can only be called in a 2-argument httpServe handler. "
                "In a 1-argument handler, the request body is already fully read. "
                "Use readBody(req) instead.\n");
        exit(1);
    }

    Net4BodyState *bs = tl_net4_body;
    if (!bs) {
        fprintf(stderr, "readBodyAll: no active body streaming state\n");
        exit(1);
    }

    // NB4-7: Verify request token.
    uint64_t tok = taida_net4_extract_body_token(req);
    if (tok != bs->request_token) {
        fprintf(stderr, "readBodyAll: request pack does not match the current active request.\n");
        exit(1);
    }

    Net3WriterState *w = tl_net3_writer;
    if (w && w->state == NET3_STATE_WEBSOCKET) {
        fprintf(stderr, "readBodyAll: cannot read HTTP body after WebSocket upgrade.\n");
        exit(1);
    }

    int fd = tl_net3_client_fd;

    bs->any_read_started = 1;

    if (bs->fully_read) {
        return taida_bytes_new_filled(0, 0);
    }

    // Aggregate all remaining body bytes (this is the only permitted aggregate path).
    size_t all_cap = 4096;
    size_t all_len = 0;
    unsigned char *all_buf = (unsigned char*)TAIDA_MALLOC(all_cap, "net_readBodyAll");

    if (bs->is_chunked) {
        char line_buf[4096];
        for (;;) {
            switch (bs->chunked_state) {
                case NET4_CHUNKED_DONE:
                    bs->fully_read = 1;
                    goto all_done;

                case NET4_CHUNKED_WAIT_SIZE: {
                    size_t llen = taida_net4_read_line(bs, fd, line_buf, sizeof(line_buf));
                    size_t s = 0, e = llen;
                    while (s < e && (line_buf[s]==' '||line_buf[s]=='\t'||line_buf[s]=='\r'||line_buf[s]=='\n')) s++;
                    while (e > s && (line_buf[e-1]==' '||line_buf[e-1]=='\t'||line_buf[e-1]=='\r'||line_buf[e-1]=='\n')) e--;
                    if (s == e) continue;
                    char hex_buf[64];
                    size_t hex_len = 0;
                    for (size_t i = s; i < e && line_buf[i] != ';' && hex_len < 63; i++) {
                        if (line_buf[i] != ' ' && line_buf[i] != '\t')
                            hex_buf[hex_len++] = line_buf[i];
                    }
                    hex_buf[hex_len] = '\0';
                    // NB4-18: Strict hex-only parse. Reject partial parse like '1g'.
                    for (size_t vi = 0; vi < hex_len; vi++) {
                        char c = hex_buf[vi];
                        if (!((c >= '0' && c <= '9') || (c >= 'a' && c <= 'f') || (c >= 'A' && c <= 'F'))) {
                            fprintf(stderr, "readBodyChunk: invalid chunk-size '%s' in chunked body\n", hex_buf);
                            exit(1);
                        }
                    }
                    if (hex_len == 0) continue; // skip empty, retry
                    unsigned long chunk_size = strtoul(hex_buf, NULL, 16);
                    if (chunk_size == 0) {
                        bs->chunked_state = NET4_CHUNKED_DONE;
                        bs->fully_read = 1;
                        if (taida_net4_drain_chunked_trailers(bs, fd) < 0) {
                            bs->fully_read = 0;
                            fprintf(stderr, "readBodyAll: chunked body protocol error\n");
                            exit(1);
                        }
                        goto all_done;
                    }
                    bs->chunked_state = NET4_CHUNKED_READ_DATA;
                    bs->chunked_remaining = (size_t)chunk_size;
                    break;
                }

                case NET4_CHUNKED_READ_DATA: {
                    if (bs->chunked_remaining == 0) {
                        bs->chunked_state = NET4_CHUNKED_WAIT_TRAILER;
                        continue;
                    }
                    // Ensure capacity.
                    while (all_len + bs->chunked_remaining > all_cap) {
                        all_cap *= 2;
                        TAIDA_REALLOC(all_buf, all_cap, "net_readBodyAll_grow");
                    }
                    size_t got = taida_net4_read_body_bytes(bs, fd, all_buf + all_len, bs->chunked_remaining);
                    // NB4-18: short read (EOF) in chunked data is a protocol error.
                    if (got == 0) {
                        fprintf(stderr, "readBodyAll: truncated chunked body — expected %zu more chunk-data bytes but got EOF\n",
                                bs->chunked_remaining);
                        free(all_buf);
                        exit(1);
                    }
                    all_len += got;
                    size_t new_rem = bs->chunked_remaining - got;
                    bs->chunked_remaining = new_rem;
                    break;
                }

                case NET4_CHUNKED_WAIT_TRAILER: {
                    // NB4-18: Read CRLF after chunk data and validate.
                    {
                        size_t tl_len2 = taida_net4_read_line(bs, fd, line_buf, sizeof(line_buf));
                        if (tl_len2 == 0) {
                            fprintf(stderr, "readBodyAll: missing CRLF after chunk data (unexpected EOF)\n");
                            free(all_buf);
                            exit(1);
                        }
                        size_t ts2 = 0, te2 = tl_len2;
                        while (ts2 < te2 && (line_buf[ts2]==' '||line_buf[ts2]=='\t'||line_buf[ts2]=='\r'||line_buf[ts2]=='\n')) ts2++;
                        while (te2 > ts2 && (line_buf[te2-1]==' '||line_buf[te2-1]=='\t'||line_buf[te2-1]=='\r'||line_buf[te2-1]=='\n')) te2--;
                        if (ts2 != te2) {
                            line_buf[tl_len2 < sizeof(line_buf)-1 ? tl_len2 : sizeof(line_buf)-1] = '\0';
                            fprintf(stderr, "readBodyAll: malformed chunk trailer — expected CRLF after chunk data, got \"%s\"\n", line_buf);
                            free(all_buf);
                            exit(1);
                        }
                    }
                    bs->chunked_state = NET4_CHUNKED_WAIT_SIZE;
                    break;
                }
            }
        }
    } else {
        // Content-Length path.
        int64_t remaining = bs->content_length - bs->bytes_consumed;
        if (remaining > 0) {
            size_t to_read = (size_t)remaining;
            if (to_read > all_cap) {
                all_cap = to_read;
                TAIDA_REALLOC(all_buf, all_cap, "net_readBodyAll_cl");
            }
            size_t got = taida_net4_read_body_bytes(bs, fd, all_buf, to_read);
            // NB4-18: EOF before Content-Length exhausted is a protocol error.
            if (got == 0 && to_read > 0) {
                fprintf(stderr, "readBodyAll: truncated body — expected %" PRId64
                        " bytes (Content-Length) but got EOF after %" PRId64 " bytes\n",
                        bs->content_length, bs->bytes_consumed);
                free(all_buf);
                exit(1);
            }
            all_len = got;
            bs->bytes_consumed += (int64_t)got;
        }
        bs->fully_read = 1;
    }

all_done:;
    taida_val result = taida_bytes_from_raw(all_buf, (taida_val)all_len);
    free(all_buf);
    return result;
}

// ── WebSocket frame write (NET4-4c) ─────────────────────────
// Server->client: FIN=1, MASK=0. Header on stack, payload via writev.
static int taida_net4_write_ws_frame(int fd, uint8_t opcode, const unsigned char *payload, size_t payload_len) {
    unsigned char header[10];
    int header_len;
    header[0] = 0x80 | opcode; // FIN=1
    if (payload_len < 126) {
        header[1] = (uint8_t)payload_len;
        header_len = 2;
    } else if (payload_len <= 65535) {
        header[1] = 126;
        header[2] = (uint8_t)(payload_len >> 8);
        header[3] = (uint8_t)(payload_len & 0xFF);
        header_len = 4;
    } else {
        header[1] = 127;
        uint64_t len64 = (uint64_t)payload_len;
        header[2] = (uint8_t)(len64 >> 56);
        header[3] = (uint8_t)(len64 >> 48);
        header[4] = (uint8_t)(len64 >> 40);
        header[5] = (uint8_t)(len64 >> 32);
        header[6] = (uint8_t)(len64 >> 24);
        header[7] = (uint8_t)(len64 >> 16);
        header[8] = (uint8_t)(len64 >> 8);
        header[9] = (uint8_t)(len64);
        header_len = 10;
    }
    // Vectored write: header + payload (no aggregate buffer).
    struct iovec iov[2];
    iov[0].iov_base = header;
    iov[0].iov_len = (size_t)header_len;
    iov[1].iov_base = (void*)payload;
    iov[1].iov_len = payload_len;
    return taida_net_writev_all(fd, iov, payload_len > 0 ? 2 : 1);
}

// ── WebSocket frame read (NET4-4c) ──────────────────────────
// Frame types returned by read_ws_frame.
#define WS_FRAME_TEXT     1
#define WS_FRAME_BINARY   2
#define WS_FRAME_PING     3
#define WS_FRAME_PONG     4
#define WS_FRAME_CLOSE    5
#define WS_FRAME_ERROR    6

typedef struct {
    int type;                // WS_FRAME_*
    unsigned char *payload;  // heap-allocated payload (caller must free)
    size_t payload_len;
    uint8_t opcode;
} WsFrameResult;

static WsFrameResult taida_net4_read_ws_frame(int fd) {
    WsFrameResult result = { WS_FRAME_ERROR, NULL, 0, 0 };
    unsigned char hdr[2];
    if (taida_net4_recv_exact(fd, hdr, 2) != 2) {
        return result;
    }
    uint8_t byte0 = hdr[0], byte1 = hdr[1];
    int fin = (byte0 & 0x80) != 0;
    uint8_t rsv = byte0 & 0x70;
    uint8_t opcode = byte0 & 0x0F;
    int masked = (byte1 & 0x80) != 0;
    uint64_t payload_len7 = byte1 & 0x7F;

    // RSV must be 0.
    if (rsv != 0) { result.type = WS_FRAME_ERROR; return result; }

    // Fragmented frames not supported.
    if (!fin) { result.type = WS_FRAME_ERROR; return result; }

    // Continuation frame without fragmentation is error.
    if (opcode == 0x0) { result.type = WS_FRAME_ERROR; return result; }

    // NB4-11: Client-to-server frames MUST be masked (RFC 6455 Section 5.1).
    if (!masked) { result.type = WS_FRAME_ERROR; return result; }

    // Determine actual payload length.
    uint64_t payload_len;
    if (payload_len7 < 126) {
        payload_len = payload_len7;
    } else if (payload_len7 == 126) {
        unsigned char ext[2];
        if (taida_net4_recv_exact(fd, ext, 2) != 2) return result;
        payload_len = ((uint64_t)ext[0] << 8) | ext[1];
    } else { // 127
        unsigned char ext[8];
        if (taida_net4_recv_exact(fd, ext, 8) != 8) return result;
        payload_len = 0;
        for (int i = 0; i < 8; i++) payload_len = (payload_len << 8) | ext[i];
        if (payload_len >> 63) { result.type = WS_FRAME_ERROR; return result; }
    }

    // Oversized payload check.
    if (payload_len > WS_MAX_PAYLOAD) { result.type = WS_FRAME_ERROR; return result; }

    // Read masking key if masked.
    uint8_t mask_key[4] = {0};
    if (masked) {
        if (taida_net4_recv_exact(fd, mask_key, 4) != 4) return result;
    }

    // NB6-9: Read payload using stack buffer for small frames (<=4KB) to avoid
    // per-frame malloc/free overhead for high-frequency small WebSocket messages.
    // Heap fallback for larger payloads.
    unsigned char stack_payload[4096];
    unsigned char *payload = NULL;
    int payload_on_heap = 0;
    if (payload_len > 0) {
        if ((size_t)payload_len <= sizeof(stack_payload)) {
            payload = stack_payload;
        } else {
            payload = (unsigned char*)TAIDA_MALLOC((size_t)payload_len, "net_ws_frame_payload");
            payload_on_heap = 1;
        }
        if (taida_net4_recv_exact(fd, payload, (size_t)payload_len) != (size_t)payload_len) {
            if (payload_on_heap) free(payload);
            return result;
        }
        // NB6-6: Unmask in-place using word-at-a-time XOR.
        // Process 4 bytes at a time to eliminate modulo per byte.
        if (masked) {
            uint32_t mask_word;
            memcpy(&mask_word, mask_key, 4);
            size_t plen = (size_t)payload_len;
            size_t i = 0;
            // Word-at-a-time loop.
            for (; i + 4 <= plen; i += 4) {
                uint32_t word;
                memcpy(&word, payload + i, 4);
                word ^= mask_word;
                memcpy(payload + i, &word, 4);
            }
            // Handle remaining 1-3 bytes.
            for (; i < plen; i++) {
                payload[i] ^= mask_key[i & 3];
            }
        }
    }

    // NB6-9: If payload was on stack, copy to heap for caller to free.
    if (payload && !payload_on_heap) {
        unsigned char *heap_copy = (unsigned char*)TAIDA_MALLOC((size_t)payload_len, "net_ws_frame_payload");
        memcpy(heap_copy, payload, (size_t)payload_len);
        payload = heap_copy;
    }
    result.payload = payload;
    result.payload_len = (size_t)payload_len;
    result.opcode = opcode;

    switch (opcode) {
        case WS_OPCODE_TEXT:   result.type = WS_FRAME_TEXT; break;
        case WS_OPCODE_BINARY: result.type = WS_FRAME_BINARY; break;
        case WS_OPCODE_CLOSE:  result.type = WS_FRAME_CLOSE; break;
        case WS_OPCODE_PING:   result.type = WS_FRAME_PING; break;
        case WS_OPCODE_PONG:   result.type = WS_FRAME_PONG; break;
        default:               result.type = WS_FRAME_ERROR; break;
    }
    return result;
}

// ── NB4-10: Validate WsConn token — sentinel + connection-scoped token ──
static int taida_net4_validate_ws_token(taida_val ws) {
    if (!taida_is_buchi_pack(ws)) return 0;
    // Check sentinel.
    taida_val id_val = taida_pack_get(ws, taida_str_hash((taida_val)"__ws_id"));
    if (id_val == 0) return 0;
    const char *id_str = (const char*)id_val;
    size_t id_len = 0;
    if (!taida_read_cstr_len_safe(id_str, 64, &id_len)) return 0;
    if (id_len != 19 || memcmp(id_str, "__v4_websocket_conn", 19) != 0) return 0;
    // Verify connection-scoped token matches active ws_token.
    Net4BodyState *bs = tl_net4_body;
    if (!bs || bs->ws_token == 0) return 0;
    taida_val tok_val = taida_pack_get(ws, taida_str_hash((taida_val)"__ws_token"));
    if ((uint64_t)tok_val != bs->ws_token) return 0;
    return 1;
}

// Make Lax[@(ws: WsConn)] with value.
static taida_val taida_net4_make_lax_ws_value(taida_val ws_pack) {
    taida_val inner = taida_pack_new(1);
    taida_pack_set_hash(inner, 0, taida_str_hash((taida_val)"ws"));
    taida_pack_set(inner, 0, ws_pack);
    taida_pack_set_tag(inner, 0, TAIDA_TAG_PACK);
    taida_retain(ws_pack);
    return taida_lax_new(inner, taida_pack_new(0));
}

// Make Lax empty for failed wsUpgrade.
static taida_val taida_net4_make_lax_ws_empty(void) {
    return taida_lax_empty(taida_pack_new(0));
}

// Make Lax[@(type, data)] for wsReceive data frame.
static taida_val taida_net4_make_lax_ws_frame_value(const char *type_str, taida_val data_val) {
    taida_val inner = taida_pack_new(2);
    taida_pack_set_hash(inner, 0, taida_str_hash((taida_val)"type"));
    taida_pack_set(inner, 0, (taida_val)taida_str_new_copy(type_str));
    taida_pack_set_tag(inner, 0, TAIDA_TAG_STR);
    taida_pack_set_hash(inner, 1, taida_str_hash((taida_val)"data"));
    taida_pack_set(inner, 1, data_val);
    // Tag the data field appropriately.
    if (TAIDA_IS_BYTES(data_val)) {
        taida_pack_set_tag(inner, 1, TAIDA_TAG_PACK); // Bytes is ptr
    } else {
        taida_pack_set_tag(inner, 1, TAIDA_TAG_STR);
    }
    return taida_lax_new(inner, taida_pack_new(0));
}

// Make Lax empty for wsReceive close / end of stream.
static taida_val taida_net4_make_lax_ws_frame_empty(void) {
    return taida_lax_empty(taida_pack_new(0));
}

// ── Helper: extract header value from request pack (case-insensitive) ──
static int taida_net4_get_header_value(taida_val req, const unsigned char *raw, size_t raw_len,
                                        const char *target_name, char *out, size_t out_cap) {
    taida_val headers = taida_pack_get(req, taida_str_hash((taida_val)"headers"));
    if (!TAIDA_IS_LIST(headers)) return 0;
    taida_val *hdr_list = (taida_val*)headers;
    taida_val hdr_count = hdr_list[2];
    size_t target_len = strlen(target_name);

    for (taida_val i = 0; i < hdr_count; i++) {
        taida_val header = hdr_list[4 + i];
        if (!taida_is_buchi_pack(header)) continue;

        taida_val name_span = taida_pack_get(header, taida_str_hash((taida_val)"name"));
        if (!taida_is_buchi_pack(name_span)) continue;
        taida_val n_start = taida_pack_get(name_span, taida_str_hash((taida_val)"start"));
        taida_val n_len = taida_pack_get(name_span, taida_str_hash((taida_val)"len"));
        if (n_start < 0 || n_len <= 0 || (size_t)(n_start + n_len) > raw_len) continue;
        if ((size_t)n_len != target_len) continue;

        int match = 1;
        for (size_t j = 0; j < target_len; j++) {
            char c = (char)raw[n_start + j];
            if (c >= 'A' && c <= 'Z') c += 32;
            char t = target_name[j];
            if (t >= 'A' && t <= 'Z') t += 32;
            if (c != t) { match = 0; break; }
        }
        if (!match) continue;

        taida_val val_span = taida_pack_get(header, taida_str_hash((taida_val)"value"));
        if (!taida_is_buchi_pack(val_span)) continue;
        taida_val v_start = taida_pack_get(val_span, taida_str_hash((taida_val)"start"));
        taida_val v_len = taida_pack_get(val_span, taida_str_hash((taida_val)"len"));
        if (v_start < 0 || v_len <= 0 || (size_t)(v_start + v_len) > raw_len) continue;

        size_t copy_len = (size_t)v_len;
        if (copy_len >= out_cap) copy_len = out_cap - 1;
        memcpy(out, raw + v_start, copy_len);
        out[copy_len] = '\0';
        return 1;
    }
    return 0;
}

// ── Helper: extract method string from request ──
static int taida_net4_get_method(taida_val req, const unsigned char *raw, size_t raw_len, char *out, size_t out_cap) {
    taida_val method_span = taida_pack_get(req, taida_str_hash((taida_val)"method"));
    if (!taida_is_buchi_pack(method_span)) return 0;
    taida_val m_start = taida_pack_get(method_span, taida_str_hash((taida_val)"start"));
    taida_val m_len = taida_pack_get(method_span, taida_str_hash((taida_val)"len"));
    if (m_start < 0 || m_len <= 0 || (size_t)(m_start + m_len) > raw_len) return 0;
    size_t copy_len = (size_t)m_len;
    if (copy_len >= out_cap) copy_len = out_cap - 1;
    memcpy(out, raw + m_start, copy_len);
    out[copy_len] = '\0';
    return 1;
}

// Case-insensitive string compare.
static int taida_net4_strcasecmp(const char *a, const char *b) {
    while (*a && *b) {
        char ca = *a, cb = *b;
        if (ca >= 'A' && ca <= 'Z') ca += 32;
        if (cb >= 'A' && cb <= 'Z') cb += 32;
        if (ca != cb) return ca - cb;
        a++; b++;
    }
    return (unsigned char)*a - (unsigned char)*b;
}

// Check if a comma-separated header value contains a token (case-insensitive).
static int taida_net4_header_contains_token(const char *value, const char *token) {
    size_t token_len = strlen(token);
    const char *p = value;
    while (*p) {
        // Skip leading whitespace and commas.
        while (*p == ' ' || *p == '\t' || *p == ',') p++;
        if (!*p) break;
        const char *start = p;
        while (*p && *p != ',') p++;
        // Trim trailing whitespace.
        const char *end = p;
        while (end > start && (end[-1] == ' ' || end[-1] == '\t')) end--;
        size_t tlen = (size_t)(end - start);
        if (tlen == token_len) {
            int match = 1;
            for (size_t i = 0; i < tlen; i++) {
                char ca = start[i], cb = token[i];
                if (ca >= 'A' && ca <= 'Z') ca += 32;
                if (cb >= 'A' && cb <= 'Z') cb += 32;
                if (ca != cb) { match = 0; break; }
            }
            if (match) return 1;
        }
    }
    return 0;
}

// ── wsUpgrade(req, writer) → Lax[@(ws: WsConn)] (NET4-4b) ──
taida_val taida_net_ws_upgrade(taida_val req, taida_val writer) {
    // Must be inside 2-arg handler.
    Net3WriterState *w = tl_net3_writer;
    if (!w) {
        fprintf(stderr, "wsUpgrade: can only be called inside a 2-argument httpServe handler\n");
        exit(1);
    }

    // Validate writer token.
    taida_net3_validate_writer(writer, "wsUpgrade");

    // NB4-10: Verify request token matches the active body state.
    {
        Net4BodyState *bs_check = tl_net4_body;
        if (bs_check) {
            uint64_t tok = taida_net4_extract_body_token(req);
            if (tok != bs_check->request_token) {
                fprintf(stderr, "wsUpgrade: request pack does not match the current active request. "
                        "The request may be stale or fabricated.\n");
                exit(1);
            }
        }
    }

    // State check: only valid in Idle state.
    switch (w->state) {
        case NET3_STATE_IDLE: break;
        case NET3_STATE_HEAD_PREPARED:
        case NET3_STATE_STREAMING:
            fprintf(stderr, "wsUpgrade: cannot upgrade after HTTP response has started. "
                    "wsUpgrade must be called before startResponse/writeChunk.\n");
            exit(1);
        case NET3_STATE_ENDED:
            fprintf(stderr, "wsUpgrade: cannot upgrade after HTTP response has ended.\n");
            exit(1);
        case NET3_STATE_WEBSOCKET:
            fprintf(stderr, "wsUpgrade: WebSocket upgrade already completed.\n");
            exit(1);
    }

    if (!taida_is_buchi_pack(req)) {
        return taida_net4_make_lax_ws_empty();
    }

    // Extract raw bytes for header value extraction.
    taida_val raw_val = taida_pack_get(req, taida_str_hash((taida_val)"raw"));
    if (!TAIDA_IS_BYTES(raw_val)) {
        return taida_net4_make_lax_ws_empty();
    }
    taida_val *raw_arr = (taida_val*)raw_val;
    taida_val raw_len = raw_arr[1];
    // Materialize raw bytes for C string comparison.
    unsigned char *raw = (unsigned char*)TAIDA_MALLOC((size_t)raw_len + 1, "net_ws_raw");
    for (taida_val i = 0; i < raw_len; i++) raw[i] = (unsigned char)raw_arr[2 + i];
    raw[raw_len] = 0;

    // Validate: must be GET.
    char method[16];
    if (!taida_net4_get_method(req, raw, (size_t)raw_len, method, sizeof(method)) ||
        taida_net4_strcasecmp(method, "GET") != 0) {
        free(raw);
        return taida_net4_make_lax_ws_empty();
    }

    // Check: no body (Content-Length must be 0 or absent, not chunked).
    taida_val cl = taida_pack_get(req, taida_str_hash((taida_val)"contentLength"));
    taida_val chunked_val = taida_pack_get(req, taida_str_hash((taida_val)"chunked"));
    if (cl > 0 || chunked_val != 0) {
        free(raw);
        return taida_net4_make_lax_ws_empty();
    }

    // Validate: Upgrade: websocket
    char hdr_buf[256];
    if (!taida_net4_get_header_value(req, raw, (size_t)raw_len, "upgrade", hdr_buf, sizeof(hdr_buf)) ||
        taida_net4_strcasecmp(hdr_buf, "websocket") != 0) {
        free(raw);
        return taida_net4_make_lax_ws_empty();
    }

    // Validate: Connection contains "Upgrade"
    if (!taida_net4_get_header_value(req, raw, (size_t)raw_len, "connection", hdr_buf, sizeof(hdr_buf)) ||
        !taida_net4_header_contains_token(hdr_buf, "Upgrade")) {
        free(raw);
        return taida_net4_make_lax_ws_empty();
    }

    // Validate: Sec-WebSocket-Version: 13
    if (!taida_net4_get_header_value(req, raw, (size_t)raw_len, "sec-websocket-version", hdr_buf, sizeof(hdr_buf))) {
        free(raw);
        return taida_net4_make_lax_ws_empty();
    }
    // Trim whitespace.
    {
        char *p = hdr_buf;
        while (*p == ' ' || *p == '\t') p++;
        if (strcmp(p, "13") != 0) {
            free(raw);
            return taida_net4_make_lax_ws_empty();
        }
    }

    // Validate: Sec-WebSocket-Key (must be present and non-empty).
    char ws_key[256];
    if (!taida_net4_get_header_value(req, raw, (size_t)raw_len, "sec-websocket-key", ws_key, sizeof(ws_key))) {
        free(raw);
        return taida_net4_make_lax_ws_empty();
    }
    // Trim key.
    {
        size_t ks = 0, ke = strlen(ws_key);
        while (ks < ke && (ws_key[ks] == ' ' || ws_key[ks] == '\t')) ks++;
        while (ke > ks && (ws_key[ke-1] == ' ' || ws_key[ke-1] == '\t')) ke--;
        if (ks > 0) memmove(ws_key, ws_key + ks, ke - ks);
        ws_key[ke - ks] = '\0';
    }
    if (ws_key[0] == '\0') {
        free(raw);
        return taida_net4_make_lax_ws_empty();
    }
    // NB4-11: RFC 6455: key must be 24 chars and decode to exactly 16 bytes.
    {
        size_t key_len = strlen(ws_key);
        if (key_len != 24) {
            free(raw);
            return taida_net4_make_lax_ws_empty();
        }
        uint8_t decoded[18]; // 16 bytes + margin
        int dec_len = taida_base64_decode(ws_key, key_len, decoded, sizeof(decoded));
        if (dec_len != 16) {
            free(raw);
            return taida_net4_make_lax_ws_empty();
        }
    }

    free(raw);

    // All validations passed. Compute accept and send 101 response.
    char *accept = taida_net4_compute_ws_accept(ws_key);

    int fd = tl_net3_client_fd;
    char response[512];
    int rlen = snprintf(response, sizeof(response),
        "HTTP/1.1 101 Switching Protocols\r\n"
        "Upgrade: websocket\r\n"
        "Connection: Upgrade\r\n"
        "Sec-WebSocket-Accept: %s\r\n"
        "\r\n", accept);
    free(accept);

    if (rlen < 0 || (size_t)rlen >= sizeof(response)) {
        return taida_net4_make_lax_ws_empty();
    }
    taida_net_send_all(fd, response, (size_t)rlen);

    // Transition to WebSocket state.
    w->state = NET3_STATE_WEBSOCKET;

    // Mark body state and set ws token.
    Net4BodyState *bs = tl_net4_body;
    uint64_t ws_tok = taida_net4_alloc_ws_token();
    if (bs) {
        bs->ws_closed = 0;
        bs->ws_token = ws_tok;
    }

    // Create WsConn BuchiPack with identity token (NB4-10).
    taida_val ws_pack = taida_pack_new(2);
    taida_pack_set_hash(ws_pack, 0, taida_str_hash((taida_val)"__ws_id"));
    taida_pack_set(ws_pack, 0, (taida_val)"__v4_websocket_conn");
    taida_pack_set_tag(ws_pack, 0, TAIDA_TAG_STR);
    taida_pack_set_hash(ws_pack, 1, taida_str_hash((taida_val)"__ws_token"));
    taida_pack_set(ws_pack, 1, (taida_val)ws_tok);
    taida_pack_set_tag(ws_pack, 1, TAIDA_TAG_INT);

    return taida_net4_make_lax_ws_value(ws_pack);
}

// ── wsSend(ws, data) → Unit (NET4-4d) ───────────────────────
taida_val taida_net_ws_send(taida_val ws, taida_val data) {
    Net3WriterState *w = tl_net3_writer;
    if (!w) {
        fprintf(stderr, "wsSend: can only be called inside a 2-argument httpServe handler\n");
        exit(1);
    }

    if (!taida_net4_validate_ws_token(ws)) {
        fprintf(stderr, "wsSend: first argument must be the WebSocket connection from wsUpgrade\n");
        exit(1);
    }

    if (w->state != NET3_STATE_WEBSOCKET) {
        fprintf(stderr, "wsSend: not in WebSocket state. Call wsUpgrade first.\n");
        exit(1);
    }

    Net4BodyState *bs = tl_net4_body;
    if (bs && bs->ws_closed) {
        fprintf(stderr, "wsSend: WebSocket connection is already closed.\n");
        exit(1);
    }

    int fd = tl_net3_client_fd;

    // Determine opcode and payload.
    uint8_t opcode;
    const unsigned char *payload;
    size_t payload_len;
    unsigned char *temp_buf = NULL;

    if (TAIDA_IS_BYTES(data)) {
        opcode = WS_OPCODE_BINARY;
        taida_val *bytes = (taida_val*)data;
        taida_val blen = bytes[1];
        payload_len = (size_t)blen;
        temp_buf = (unsigned char*)TAIDA_MALLOC(payload_len + 1, "net_ws_send_bytes");
        for (taida_val i = 0; i < blen; i++) temp_buf[i] = (unsigned char)bytes[2 + i];
        payload = temp_buf;
    } else {
        // Assume Str -> text frame.
        opcode = WS_OPCODE_TEXT;
        const char *s = (const char*)data;
        size_t slen = 0;
        if (!taida_read_cstr_len_safe(s, 64 * 1024 * 1024, &slen)) {
            fprintf(stderr, "wsSend: data must be Str (text frame) or Bytes (binary frame)\n");
            exit(1);
        }
        payload = (const unsigned char*)s;
        payload_len = slen;
    }

    taida_net4_write_ws_frame(fd, opcode, payload, payload_len);
    if (temp_buf) free(temp_buf);

    return 0; // Unit
}

// ── wsReceive(ws) → Lax[@(type, data)] (NET4-4d) ────────────
taida_val taida_net_ws_receive(taida_val ws) {
    Net3WriterState *w = tl_net3_writer;
    if (!w) {
        fprintf(stderr, "wsReceive: can only be called inside a 2-argument httpServe handler\n");
        exit(1);
    }

    if (!taida_net4_validate_ws_token(ws)) {
        fprintf(stderr, "wsReceive: first argument must be the WebSocket connection from wsUpgrade\n");
        exit(1);
    }

    if (w->state != NET3_STATE_WEBSOCKET) {
        fprintf(stderr, "wsReceive: not in WebSocket state. Call wsUpgrade first.\n");
        exit(1);
    }

    Net4BodyState *bs = tl_net4_body;
    if (bs && bs->ws_closed) {
        return taida_net4_make_lax_ws_frame_empty();
    }

    int fd = tl_net3_client_fd;

    // Loop to handle ping/pong transparently.
    for (;;) {
        WsFrameResult frame = taida_net4_read_ws_frame(fd);

        switch (frame.type) {
            case WS_FRAME_TEXT: {
                // Text frame: return data as Str (parity with Interpreter).
                char *text = NULL;
                if (frame.payload_len > 0) {
                    text = (char*)TAIDA_MALLOC(frame.payload_len + 1, "net_ws_text");
                    memcpy(text, frame.payload, frame.payload_len);
                    text[frame.payload_len] = '\0';
                } else {
                    text = taida_str_new_copy("");
                }
                free(frame.payload);
                taida_val data_val = (taida_val)text;
                return taida_net4_make_lax_ws_frame_value("text", data_val);
            }

            case WS_FRAME_BINARY: {
                taida_val bytes = taida_bytes_from_raw(frame.payload, (taida_val)frame.payload_len);
                free(frame.payload);
                return taida_net4_make_lax_ws_frame_value("binary", bytes);
            }

            case WS_FRAME_PING: {
                // Auto pong: send pong with same payload.
                taida_net4_write_ws_frame(fd, WS_OPCODE_PONG,
                    frame.payload ? frame.payload : (unsigned char*)"",
                    frame.payload_len);
                if (frame.payload) free(frame.payload);
                continue; // Next frame.
            }

            case WS_FRAME_PONG: {
                // Unsolicited pong: ignore.
                if (frame.payload) free(frame.payload);
                continue;
            }

            case WS_FRAME_CLOSE: {
                // v5 close code extraction (NET5-0d).
                if (frame.payload_len == 0) {
                    // No status code: reply with empty close payload.
                    if (bs && !bs->ws_closed) {
                        taida_net4_write_ws_frame(fd, WS_OPCODE_CLOSE, (unsigned char*)"", 0);
                    }
                    if (bs) {
                        bs->ws_closed = 1;
                        bs->ws_close_code = 1005; // No Status Rcvd
                    }
                    if (frame.payload) free(frame.payload);
                    return taida_net4_make_lax_ws_frame_empty();
                } else if (frame.payload_len == 1) {
                    // 1-byte close payload is malformed.
                    unsigned char close_1002[2] = { 0x03, 0xEA };
                    taida_net4_write_ws_frame(fd, WS_OPCODE_CLOSE, close_1002, 2);
                    if (bs) bs->ws_closed = 1;
                    if (frame.payload) free(frame.payload);
                    fprintf(stderr, "wsReceive: protocol error: malformed close frame (1-byte payload)\n");
                    exit(1);
                } else {
                    // 2+ bytes: first 2 bytes are the close code (big-endian).
                    uint16_t code = ((uint16_t)frame.payload[0] << 8) | (uint16_t)frame.payload[1];
                    // Validate close code (RFC 6455 Section 7.4).
                    // 1000-1003: standard, 1007-1014: IANA-registered,
                    // 3000-4999: reserved for libraries/apps/private use.
                    int valid_code = (code >= 1000 && code <= 1003) ||
                                     (code >= 1007 && code <= 1014) ||
                                     (code >= 3000 && code <= 4999);
                    if (!valid_code) {
                        unsigned char close_1002[2] = { 0x03, 0xEA };
                        taida_net4_write_ws_frame(fd, WS_OPCODE_CLOSE, close_1002, 2);
                        if (bs) bs->ws_closed = 1;
                        free(frame.payload);
                        fprintf(stderr, "wsReceive: protocol error: invalid close code %u\n", (unsigned)code);
                        exit(1);
                    }
                    // Validate reason UTF-8 if present.
                    // Strict UTF-8 validation: reject overlong sequences, surrogate
                    // halves (U+D800..U+DFFF), and code points > U+10FFFF to match
                    // Interpreter (std::str::from_utf8) and JS (decode+re-encode).
                    if (frame.payload_len > 2) {
                        size_t rlen = frame.payload_len - 2;
                        unsigned char *reason = frame.payload + 2;
                        size_t i = 0;
                        int utf8_ok = 1;
                        while (i < rlen && utf8_ok) {
                            unsigned char c = reason[i];
                            if (c < 0x80) {
                                i++;
                            } else if ((c & 0xE0) == 0xC0) {
                                // 2-byte: must have 1 continuation, code point >= 0x80
                                if (i + 1 >= rlen || (reason[i+1] & 0xC0) != 0x80) { utf8_ok = 0; break; }
                                uint32_t cp = ((uint32_t)(c & 0x1F) << 6) | (uint32_t)(reason[i+1] & 0x3F);
                                if (cp < 0x80) { utf8_ok = 0; break; } // overlong
                                i += 2;
                            } else if ((c & 0xF0) == 0xE0) {
                                // 3-byte: must have 2 continuations, cp >= 0x800, not surrogate
                                if (i + 2 >= rlen || (reason[i+1] & 0xC0) != 0x80 || (reason[i+2] & 0xC0) != 0x80) { utf8_ok = 0; break; }
                                uint32_t cp = ((uint32_t)(c & 0x0F) << 12) | ((uint32_t)(reason[i+1] & 0x3F) << 6) | (uint32_t)(reason[i+2] & 0x3F);
                                if (cp < 0x800) { utf8_ok = 0; break; } // overlong
                                if (cp >= 0xD800 && cp <= 0xDFFF) { utf8_ok = 0; break; } // surrogate
                                i += 3;
                            } else if ((c & 0xF8) == 0xF0) {
                                // 4-byte: must have 3 continuations, cp >= 0x10000, cp <= 0x10FFFF
                                if (i + 3 >= rlen || (reason[i+1] & 0xC0) != 0x80 || (reason[i+2] & 0xC0) != 0x80 || (reason[i+3] & 0xC0) != 0x80) { utf8_ok = 0; break; }
                                uint32_t cp = ((uint32_t)(c & 0x07) << 18) | ((uint32_t)(reason[i+1] & 0x3F) << 12) | ((uint32_t)(reason[i+2] & 0x3F) << 6) | (uint32_t)(reason[i+3] & 0x3F);
                                if (cp < 0x10000) { utf8_ok = 0; break; } // overlong
                                if (cp > 0x10FFFF) { utf8_ok = 0; break; } // out of range
                                i += 4;
                            } else {
                                utf8_ok = 0; break; // invalid lead byte
                            }
                        }
                        if (!utf8_ok) {
                            unsigned char close_1002[2] = { 0x03, 0xEA };
                            taida_net4_write_ws_frame(fd, WS_OPCODE_CLOSE, close_1002, 2);
                            if (bs) bs->ws_closed = 1;
                            free(frame.payload);
                            fprintf(stderr, "wsReceive: protocol error: invalid UTF-8 in close reason\n");
                            exit(1);
                        }
                    }
                    // Valid close: echo the code in the reply.
                    unsigned char reply[2] = { (unsigned char)(code >> 8), (unsigned char)(code & 0xFF) };
                    if (bs && !bs->ws_closed) {
                        taida_net4_write_ws_frame(fd, WS_OPCODE_CLOSE, reply, 2);
                    }
                    if (bs) {
                        bs->ws_closed = 1;
                        bs->ws_close_code = (int64_t)code;
                    }
                    free(frame.payload);
                    return taida_net4_make_lax_ws_frame_empty();
                }
            }

            case WS_FRAME_ERROR:
            default: {
                if (frame.payload) free(frame.payload);
                // Send close frame with protocol error (1002).
                unsigned char close_payload[2] = { 0x03, 0xEA }; // 1002
                taida_net4_write_ws_frame(fd, WS_OPCODE_CLOSE, close_payload, 2);
                if (bs) bs->ws_closed = 1;
                fprintf(stderr, "wsReceive: protocol error\n");
                exit(1);
            }
        }
    }
}

// ── wsClose(ws, code) → Unit (NET4-4d, v5 revision) ────────────────
// v5: wsClose(ws) or wsClose(ws, code) → Unit.
// 2nd arg (code): 0 = default 1000 (Normal Closure), otherwise explicit close code.
// Valid codes: 1000-4999 excluding reserved 1004, 1005, 1006, 1015.
taida_val taida_net_ws_close(taida_val ws, taida_val code_val) {
    Net3WriterState *w = tl_net3_writer;
    if (!w) {
        fprintf(stderr, "wsClose: can only be called inside a 2-argument httpServe handler\n");
        exit(1);
    }

    if (!taida_net4_validate_ws_token(ws)) {
        fprintf(stderr, "wsClose: first argument must be the WebSocket connection from wsUpgrade\n");
        exit(1);
    }

    if (w->state != NET3_STATE_WEBSOCKET) {
        fprintf(stderr, "wsClose: not in WebSocket state. Call wsUpgrade first.\n");
        exit(1);
    }

    Net4BodyState *bs = tl_net4_body;

    // Idempotent: no-op if already closed.
    if (bs && bs->ws_closed) {
        return 0; // Unit
    }

    // v5: Determine close code from 2nd argument.
    // code_val is a raw Int (lowering passes 0 for default, or the literal value).
    int64_t close_code_i64 = (int64_t)code_val;

    uint16_t close_code;
    if (close_code_i64 == 0) {
        close_code = 1000; // default: Normal Closure
    } else {
        // Validate close code range.
        if (close_code_i64 < 1000 || close_code_i64 > 4999) {
            fprintf(stderr, "wsClose: close code must be 1000-4999, got %lld\n", (long long)close_code_i64);
            exit(1);
        }
        // Reserved codes that must not be sent.
        if (close_code_i64 == 1004 || close_code_i64 == 1005 || close_code_i64 == 1006 || close_code_i64 == 1015) {
            fprintf(stderr, "wsClose: close code %lld is reserved and cannot be sent\n", (long long)close_code_i64);
            exit(1);
        }
        close_code = (uint16_t)close_code_i64;
    }

    int fd = tl_net3_client_fd;

    // Send close frame with the specified close code.
    unsigned char close_payload[2] = { (unsigned char)(close_code >> 8), (unsigned char)(close_code & 0xFF) };
    taida_net4_write_ws_frame(fd, WS_OPCODE_CLOSE, close_payload, 2);

    if (bs) bs->ws_closed = 1;

    return 0; // Unit
}

// v5: wsCloseCode(ws) → Int (NET5-0d)
// Returns the close code received from the peer's close frame.
// 0 = no close frame received yet, 1005 = no status code, 1000-4999 = peer code.
taida_val taida_net_ws_close_code(taida_val ws) {
    Net3WriterState *w = tl_net3_writer;
    if (!w) {
        fprintf(stderr, "wsCloseCode: can only be called inside a 2-argument httpServe handler\n");
        exit(1);
    }

    if (!taida_net4_validate_ws_token(ws)) {
        fprintf(stderr, "wsCloseCode: first argument must be the WebSocket connection from wsUpgrade\n");
        exit(1);
    }

    if (w->state != NET3_STATE_WEBSOCKET) {
        fprintf(stderr, "wsCloseCode: not in WebSocket state. Call wsUpgrade first.\n");
        exit(1);
    }

    Net4BodyState *bs = tl_net4_body;
    int64_t code = (bs) ? bs->ws_close_code : 0;
    return (taida_val)code;
}

// Validate that the writer argument is a genuine BuchiPack token with
// __writer_id === "__v3_streaming_writer" (parity with Interpreter/JS).
static void taida_net3_validate_writer(taida_val writer, const char *api_name) {
    if (!taida_is_buchi_pack(writer)) {
        fprintf(stderr, "%s: first argument must be the writer provided by httpServe\n", api_name);
        exit(1);
    }
    taida_val id_val = taida_pack_get(writer, taida_str_hash((taida_val)"__writer_id"));
    if (id_val == 0) {
        fprintf(stderr, "%s: first argument must be the writer provided by httpServe\n", api_name);
        exit(1);
    }
    const char *id_str = (const char*)id_val;
    size_t id_len = 0;
    if (!taida_read_cstr_len_safe(id_str, 64, &id_len) ||
        id_len != 21 || memcmp(id_str, "__v3_streaming_writer", 21) != 0) {
        fprintf(stderr, "%s: first argument must be the writer provided by httpServe\n", api_name);
        exit(1);
    }
}

// Create a writer BuchiPack token for 2-arg handler.
// Contains __writer_id sentinel field (parity with Interpreter/JS).
static taida_val taida_net3_create_writer_token(void) {
    taida_val pack = taida_pack_new(1);
    taida_pack_set_hash(pack, 0, taida_str_hash((taida_val)"__writer_id"));
    taida_pack_set(pack, 0, (taida_val)"__v3_streaming_writer");
    taida_pack_set_tag(pack, 0, TAIDA_TAG_STR);
    return pack;
}

// ── NET2-5c: Thread pool structures ─────────────────────────────
// Shared state for the thread pool: a mutex-protected queue of client fds.
// Each worker thread pulls a client fd, processes the keep-alive loop, then
// returns to wait for the next fd.

typedef struct {
    int client_fd;
    struct sockaddr_in peer_addr;
} NetClientSlot;

typedef struct {
    // Shared mutable state (protected by mutex)
    pthread_mutex_t mutex;
    pthread_cond_t  cond_available;  // signal workers: new fd or shutdown
    pthread_cond_t  cond_done;       // signal main: a worker finished

    // Queue of pending client fds
    NetClientSlot *queue;
    int queue_cap;
    int queue_head;
    int queue_tail;
    int queue_count;

    // Global request counter (atomic via mutex)
    int64_t request_count;
    int64_t max_requests;

    // Active connection count (for maxConnections enforcement)
    int active_connections;

    // Shutdown flag
    int shutdown;

    // Handler and timeout
    taida_val handler;
    int64_t timeout_ms;

    // NET3-5a: handler arity (1 = one-shot, 2 = streaming, -1 = unknown/runtime detect)
    int handler_arity;

    // NET5-4a: TLS context (NULL = plaintext, non-NULL = TLS).
    OSSL_SSL_CTX *ssl_ctx;
} NetThreadPool;

static void net_pool_init(NetThreadPool *pool, int queue_cap, taida_val handler, int64_t max_requests, int64_t timeout_ms, int handler_arity) {
    pthread_mutex_init(&pool->mutex, NULL);
    pthread_cond_init(&pool->cond_available, NULL);
    pthread_cond_init(&pool->cond_done, NULL);
    pool->queue_cap = queue_cap;
    pool->queue = (NetClientSlot*)TAIDA_MALLOC(sizeof(NetClientSlot) * (size_t)queue_cap, "net_pool_queue");
    pool->queue_head = 0;
    pool->queue_tail = 0;
    pool->queue_count = 0;
    pool->request_count = 0;
    pool->max_requests = max_requests;
    pool->active_connections = 0;
    pool->shutdown = 0;
    pool->handler = handler;
    pool->timeout_ms = timeout_ms;
    pool->handler_arity = handler_arity;
    pool->ssl_ctx = NULL; // NET5-4a: set by httpServe if TLS configured
}

static void net_pool_destroy(NetThreadPool *pool) {
    pthread_mutex_destroy(&pool->mutex);
    pthread_cond_destroy(&pool->cond_available);
    pthread_cond_destroy(&pool->cond_done);
    free(pool->queue);
}

// Enqueue a client fd. Returns 0 on success, -1 if queue full.
static int net_pool_enqueue(NetThreadPool *pool, int fd, struct sockaddr_in addr) {
    if (pool->queue_count >= pool->queue_cap) return -1;
    pool->queue[pool->queue_tail].client_fd = fd;
    pool->queue[pool->queue_tail].peer_addr = addr;
    pool->queue_tail = (pool->queue_tail + 1) % pool->queue_cap;
    pool->queue_count++;
    return 0;
}

// Dequeue a client fd. Returns 0 on success, -1 if empty.
static int net_pool_dequeue(NetThreadPool *pool, NetClientSlot *out) {
    if (pool->queue_count <= 0) return -1;
    *out = pool->queue[pool->queue_head];
    pool->queue_head = (pool->queue_head + 1) % pool->queue_cap;
    pool->queue_count--;
    return 0;
}

// Check if the global request limit has been reached (call under mutex).
static int net_pool_requests_exhausted(NetThreadPool *pool) {
    return (pool->max_requests > 0 && pool->request_count >= pool->max_requests);
}

// ── NET2-5a/5b/5c: Worker thread — keep-alive loop per connection ──
static void *net_worker_thread(void *arg) {
    NetThreadPool *pool = (NetThreadPool*)arg;

    for (;;) {
        NetClientSlot slot;

        // Wait for a client fd or shutdown
        pthread_mutex_lock(&pool->mutex);
        while (pool->queue_count == 0 && !pool->shutdown) {
            pthread_cond_wait(&pool->cond_available, &pool->mutex);
        }
        if (pool->shutdown && pool->queue_count == 0) {
            pthread_mutex_unlock(&pool->mutex);
            break;
        }
        net_pool_dequeue(pool, &slot);
        pool->active_connections++;
        pthread_mutex_unlock(&pool->mutex);

        int client_fd = slot.client_fd;
        struct sockaddr_in peer_addr = slot.peer_addr;

        char host_buf[INET_ADDRSTRLEN] = {0};
        const char *peer_host = inet_ntop(AF_INET, &peer_addr.sin_addr, host_buf, sizeof(host_buf));
        if (!peer_host) peer_host = "";
        int peer_port = (int)ntohs(peer_addr.sin_port);

        // Set read timeout on client socket
        int64_t effective_timeout = (pool->timeout_ms > 0) ? pool->timeout_ms : 5000;
        {
            struct timeval tv;
            tv.tv_sec = (long)(effective_timeout / 1000);
            tv.tv_usec = (long)((effective_timeout % 1000) * 1000);
            setsockopt(client_fd, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv));
            // Also set write timeout for TLS handshake and writes.
            setsockopt(client_fd, SOL_SOCKET, SO_SNDTIMEO, &tv, sizeof(tv));
        }

        // NET5-4a: TLS handshake if pool has SSL_CTX.
        OSSL_SSL *conn_ssl = NULL;
        if (pool->ssl_ctx) {
            conn_ssl = taida_tls_handshake(pool->ssl_ctx, client_fd);
            if (!conn_ssl) {
                // NET5-0c: handshake failure = close connection, don't call handler.
                close(client_fd);
                pthread_mutex_lock(&pool->mutex);
                pool->active_connections--;
                pthread_cond_signal(&pool->cond_done);
                pthread_mutex_unlock(&pool->mutex);
                continue;
            }
        }
        tl_ssl = conn_ssl;

        // Per-connection scratch buffer (allocated once, reused via advance)
        #define NET_MAX_REQUEST_BUF 1048576
        size_t buf_cap = 8192;
        unsigned char *buf = (unsigned char*)TAIDA_MALLOC(buf_cap, "net_worker_buf");
        size_t total_read = 0;

        // ── Keep-alive loop ──
        for (;;) {

            // Phase 1: Read until HTTP head is complete
            // NB2-19: Parse once, reuse result for keepAlive + request pack building.
            // NB2-9: Properly release parse_result / parse_bytes to prevent memory leak.
            int head_complete = 0;
            size_t head_consumed = 0;
            int64_t content_length = 0;
            int is_chunked = 0;
            int head_malformed = 0;
            taida_val parse_result = 0;  // retained across head+body for single-parse reuse
            taida_val parse_inner = 0;   // inner pack from parse_result

            while (total_read < NET_MAX_REQUEST_BUF) {
                // Try to parse what we have so far
                if (total_read > 3) {
                    int found_end = 0;
                    for (size_t i = 0; i + 3 < total_read; i++) {
                        if (buf[i] == '\r' && buf[i+1] == '\n' && buf[i+2] == '\r' && buf[i+3] == '\n') {
                            found_end = 1;
                            head_consumed = i + 4;
                            break;
                        }
                    }
                    if (found_end) {
                        head_complete = 1;
                        taida_val parse_bytes = taida_bytes_from_raw(buf, (taida_val)total_read);
                        parse_result = taida_net_http_parse_request_head(parse_bytes);
                        taida_val throw_val = taida_pack_get(parse_result, taida_str_hash((taida_val)"throw"));
                        if (throw_val != 0) {
                            head_malformed = 1;
                            taida_release(parse_bytes);
                            taida_release(parse_result);
                            parse_result = 0;
                            break;
                        }
                        parse_inner = taida_pack_get(parse_result, taida_str_hash((taida_val)"__value"));
                        if (parse_inner != 0 && taida_is_buchi_pack(parse_inner)) {
                            content_length = taida_pack_get(parse_inner, taida_str_hash((taida_val)"contentLength"));
                            head_consumed = (size_t)taida_pack_get(parse_inner, taida_str_hash((taida_val)"consumed"));
                            taida_val chunked_val = taida_pack_get(parse_inner, taida_str_hash((taida_val)"chunked"));
                            is_chunked = (chunked_val != 0) ? 1 : 0;
                        }
                        taida_release(parse_bytes);
                        break;
                    }
                }

                // Read more data
                if (total_read >= buf_cap) {
                    size_t new_cap = buf_cap * 2;
                    if (new_cap > NET_MAX_REQUEST_BUF) new_cap = NET_MAX_REQUEST_BUF;
                    TAIDA_REALLOC(buf, new_cap, "net_worker_head");
                    buf_cap = new_cap;
                }
                ssize_t n = taida_tls_recv(client_fd, buf + total_read, buf_cap - total_read);
                if (n <= 0) {
                    // EOF or timeout — partial head gets 400 (parity with interpreter)
                    if (total_read > 0) {
                        const char *bad = "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                        taida_net_send_all(client_fd, bad, strlen(bad));
                        pthread_mutex_lock(&pool->mutex);
                        if (!net_pool_requests_exhausted(pool)) {
                            pool->request_count++;
                        }
                        pthread_mutex_unlock(&pool->mutex);
                    }
                    goto conn_done;
                }
                total_read += (size_t)n;
            }

            if (!head_complete || head_malformed) {
                const char *bad = "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                taida_net_send_all(client_fd, bad, strlen(bad));
                pthread_mutex_lock(&pool->mutex);
                if (!net_pool_requests_exhausted(pool)) {
                    pool->request_count++;
                }
                pthread_mutex_unlock(&pool->mutex);
                break; // close connection
            }

            // Head is complete — this counts as a real request.
            pthread_mutex_lock(&pool->mutex);
            if (net_pool_requests_exhausted(pool)) {
                pthread_mutex_unlock(&pool->mutex);
                const char *unavail = "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                taida_net_send_all(client_fd, unavail, strlen(unavail));
                if (parse_result) taida_release(parse_result);
                goto conn_done;
            }
            pool->request_count++;
            pthread_mutex_unlock(&pool->mutex);

            // NET4: Detect handler arity before body reading.
            // 2-arg handler = body-deferred (v4), 1-arg = eager body read (v2).
            int keep_alive = 1;
            size_t wire_consumed = head_consumed; // default for 2-arg deferred
            int skip_buffer_advance = 0; // NB5-24: set by 2-arg path to skip shared advance

            if (pool->handler_arity >= 2) {
                // ── v4 2-arg handler path: body-deferred ──
                // Do NOT eagerly read body. raw = head only.

                // Determine keep-alive from head.
                taida_val http_minor = 1;
                taida_val parsed_headers = 0;
                if (parse_inner != 0 && taida_is_buchi_pack(parse_inner)) {
                    taida_val ver = taida_pack_get(parse_inner, taida_str_hash((taida_val)"version"));
                    if (ver != 0 && taida_is_buchi_pack(ver)) {
                        http_minor = taida_pack_get(ver, taida_str_hash((taida_val)"minor"));
                    }
                    parsed_headers = taida_pack_get(parse_inner, taida_str_hash((taida_val)"headers"));
                }
                keep_alive = taida_net_determine_keep_alive(buf, head_consumed, parsed_headers, http_minor);

                // Capture leftover body bytes already in buf (beyond head).
                size_t leftover_len = (total_read > head_consumed) ? (total_read - head_consumed) : 0;
                unsigned char *leftover = NULL;
                if (leftover_len > 0) {
                    leftover = (unsigned char*)TAIDA_MALLOC(leftover_len, "net_v4_leftover");
                    memcpy(leftover, buf + head_consumed, leftover_len);
                }

                // Create body streaming state.
                Net4BodyState body_state;
                memset(&body_state, 0, sizeof(body_state));
                body_state.is_chunked = is_chunked;
                body_state.content_length = content_length;
                body_state.bytes_consumed = 0;
                body_state.fully_read = (!is_chunked && content_length == 0) ? 1 : 0;
                body_state.any_read_started = 0;
                body_state.leftover = leftover;
                body_state.leftover_len = leftover_len;
                body_state.leftover_pos = 0;
                body_state.chunked_state = NET4_CHUNKED_WAIT_SIZE;
                body_state.chunked_remaining = 0;
                body_state.request_token = taida_net4_alloc_token();
                body_state.ws_closed = 0;
                body_state.ws_token = 0;
                body_state.ws_close_code = 0; // v5: no close frame received yet

                // Build request pack (head only, body = empty span).
                taida_val raw_bytes = taida_bytes_from_raw(buf, (taida_val)head_consumed);
                // 15 fields: raw, method, path, query, version, headers, body, bodyOffset,
                //            contentLength, remoteHost, remotePort, keepAlive, chunked,
                //            __body_stream, __body_token
                taida_val request = taida_pack_new(15);
                taida_pack_set_hash(request, 0, taida_str_hash((taida_val)"raw"));
                taida_pack_set(request, 0, raw_bytes);
                taida_pack_set_tag(request, 0, TAIDA_TAG_PACK);

                if (parse_inner != 0 && taida_is_buchi_pack(parse_inner)) {
                    taida_val method_v = taida_pack_get(parse_inner, taida_str_hash((taida_val)"method"));
                    taida_pack_set_hash(request, 1, taida_str_hash((taida_val)"method"));
                    taida_pack_set(request, 1, method_v);
                    taida_pack_set_tag(request, 1, TAIDA_TAG_PACK);
                    if (method_v > 4096) taida_retain(method_v);

                    taida_val path_v = taida_pack_get(parse_inner, taida_str_hash((taida_val)"path"));
                    taida_pack_set_hash(request, 2, taida_str_hash((taida_val)"path"));
                    taida_pack_set(request, 2, path_v);
                    taida_pack_set_tag(request, 2, TAIDA_TAG_PACK);
                    if (path_v > 4096) taida_retain(path_v);

                    taida_val query_v = taida_pack_get(parse_inner, taida_str_hash((taida_val)"query"));
                    taida_pack_set_hash(request, 3, taida_str_hash((taida_val)"query"));
                    taida_pack_set(request, 3, query_v);
                    taida_pack_set_tag(request, 3, TAIDA_TAG_PACK);
                    if (query_v > 4096) taida_retain(query_v);

                    taida_val version_v = taida_pack_get(parse_inner, taida_str_hash((taida_val)"version"));
                    taida_pack_set_hash(request, 4, taida_str_hash((taida_val)"version"));
                    taida_pack_set(request, 4, version_v);
                    taida_pack_set_tag(request, 4, TAIDA_TAG_PACK);
                    if (version_v > 4096) taida_retain(version_v);

                    taida_val headers_v = taida_pack_get(parse_inner, taida_str_hash((taida_val)"headers"));
                    taida_pack_set_hash(request, 5, taida_str_hash((taida_val)"headers"));
                    taida_pack_set(request, 5, headers_v);
                    taida_pack_set_tag(request, 5, TAIDA_TAG_LIST);
                    if (headers_v > 4096) taida_retain(headers_v);
                } else {
                    taida_pack_set_hash(request, 1, taida_str_hash((taida_val)"method"));
                    taida_pack_set(request, 1, taida_net_make_span(0, 0));
                    taida_pack_set_tag(request, 1, TAIDA_TAG_PACK);
                    taida_pack_set_hash(request, 2, taida_str_hash((taida_val)"path"));
                    taida_pack_set(request, 2, taida_net_make_span(0, 0));
                    taida_pack_set_tag(request, 2, TAIDA_TAG_PACK);
                    taida_pack_set_hash(request, 3, taida_str_hash((taida_val)"query"));
                    taida_pack_set(request, 3, taida_net_make_span(0, 0));
                    taida_pack_set_tag(request, 3, TAIDA_TAG_PACK);
                    taida_val ver = taida_pack_new(2);
                    taida_pack_set_hash(ver, 0, taida_str_hash((taida_val)"major"));
                    taida_pack_set(ver, 0, 1);
                    taida_pack_set_hash(ver, 1, taida_str_hash((taida_val)"minor"));
                    taida_pack_set(ver, 1, 1);
                    taida_pack_set_hash(request, 4, taida_str_hash((taida_val)"version"));
                    taida_pack_set(request, 4, ver);
                    taida_pack_set_tag(request, 4, TAIDA_TAG_PACK);
                    taida_pack_set_hash(request, 5, taida_str_hash((taida_val)"headers"));
                    taida_pack_set(request, 5, taida_list_new());
                    taida_pack_set_tag(request, 5, TAIDA_TAG_LIST);
                }
                // v4: body span is empty (body not yet read).
                taida_pack_set_hash(request, 6, taida_str_hash((taida_val)"body"));
                taida_pack_set(request, 6, taida_net_make_span(0, 0));
                taida_pack_set_tag(request, 6, TAIDA_TAG_PACK);
                taida_pack_set_hash(request, 7, taida_str_hash((taida_val)"bodyOffset"));
                taida_pack_set(request, 7, (taida_val)head_consumed);
                taida_pack_set_hash(request, 8, taida_str_hash((taida_val)"contentLength"));
                taida_pack_set(request, 8, (taida_val)content_length);
                taida_pack_set_hash(request, 9, taida_str_hash((taida_val)"remoteHost"));
                taida_pack_set(request, 9, (taida_val)taida_str_new_copy(peer_host));
                taida_pack_set_tag(request, 9, TAIDA_TAG_STR);
                taida_pack_set_hash(request, 10, taida_str_hash((taida_val)"remotePort"));
                taida_pack_set(request, 10, (taida_val)peer_port);
                taida_pack_set_hash(request, 11, taida_str_hash((taida_val)"keepAlive"));
                taida_pack_set(request, 11, keep_alive ? 1 : 0);
                taida_pack_set_tag(request, 11, TAIDA_TAG_BOOL);
                taida_pack_set_hash(request, 12, taida_str_hash((taida_val)"chunked"));
                taida_pack_set(request, 12, is_chunked ? 1 : 0);
                taida_pack_set_tag(request, 12, TAIDA_TAG_BOOL);
                // v4 sentinel + token.
                taida_pack_set_hash(request, 13, taida_str_hash((taida_val)"__body_stream"));
                taida_pack_set(request, 13, (taida_val)"__v4_body_stream");
                taida_pack_set_tag(request, 13, TAIDA_TAG_STR);
                taida_pack_set_hash(request, 14, taida_str_hash((taida_val)"__body_token"));
                taida_pack_set(request, 14, (taida_val)body_state.request_token);

                if (parse_result) { taida_release(parse_result); parse_result = 0; }

                // Create writer state.
                Net3WriterState writer_state;
                writer_state.state = NET3_STATE_IDLE;
                writer_state.pending_status = 200;
                writer_state.sse_mode = 0;
                writer_state.header_count = 0;

                // Set thread-local context.
                tl_net3_writer = &writer_state;
                tl_net3_client_fd = client_fd;
                tl_net4_body = &body_state;

                taida_val writer_token = taida_net3_create_writer_token();
                taida_val response = taida_invoke_callback2(pool->handler, request, writer_token);

                // Clear thread-local context.
                tl_net3_writer = NULL;
                tl_net3_client_fd = -1;
                tl_net4_body = NULL;

                // ── v4: WebSocket auto-close on handler return ──
                if (writer_state.state == NET3_STATE_WEBSOCKET) {
                    if (!body_state.ws_closed) {
                        unsigned char close_payload[2] = { 0x03, 0xE8 }; // 1000
                        taida_net4_write_ws_frame(client_fd, WS_OPCODE_CLOSE, close_payload, 2);
                    }
                    taida_release(request);
                    taida_release(writer_token);
                    taida_release(response);
                    if (leftover) free(leftover);
                    // WebSocket: never return to keep-alive.
                    // Check request limit.
                    pthread_mutex_lock(&pool->mutex);
                    int limit_hit = net_pool_requests_exhausted(pool);
                    pthread_mutex_unlock(&pool->mutex);
                    total_read = 0;
                    if (limit_hit) {
                        // Signal shutdown.
                        pthread_mutex_lock(&pool->mutex);
                        pool->shutdown = 1;
                        pthread_cond_broadcast(&pool->cond_available);
                        pthread_mutex_unlock(&pool->mutex);
                    }
                    break; // Close connection.
                }

                if (writer_state.state == NET3_STATE_IDLE) {
                    // One-shot fallback.
                    taida_val effective_response = response;
                    int need_default = 1;
                    if (response > 4096 && taida_is_buchi_pack(response)) {
                        taida_val status_val = taida_pack_get(response, taida_str_hash((taida_val)"status"));
                        taida_val body_val = taida_pack_get(response, taida_str_hash((taida_val)"body"));
                        if (status_val != 0 || body_val != 0) need_default = 0;
                    }
                    if (need_default && (response == 0 || !taida_is_buchi_pack(response))) {
                        effective_response = taida_pack_new(3);
                        taida_pack_set_hash(effective_response, 0, taida_str_hash((taida_val)"status"));
                        taida_pack_set(effective_response, 0, 200);
                        taida_pack_set_hash(effective_response, 1, taida_str_hash((taida_val)"headers"));
                        taida_pack_set(effective_response, 1, taida_list_new());
                        taida_pack_set_tag(effective_response, 1, TAIDA_TAG_LIST);
                        taida_pack_set_hash(effective_response, 2, taida_str_hash((taida_val)"body"));
                        taida_pack_set(effective_response, 2, (taida_val)"");
                        taida_pack_set_tag(effective_response, 2, TAIDA_TAG_STR);
                    }
                    // NB6-1: Scatter-gather send — head and body as separate buffers.
                    if (taida_net_send_response_scatter(client_fd, effective_response) != 0) {
                        const char *fallback = "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                        taida_net_send_all(client_fd, fallback, strlen(fallback));
                    }
                    if (need_default && effective_response != response) taida_release(effective_response);
                } else {
                    // Streaming was started.
                    if (writer_state.state != NET3_STATE_ENDED) {
                        int auto_end_failed = 0;
                        if (writer_state.state == NET3_STATE_HEAD_PREPARED) {
                            if (taida_net3_commit_head(client_fd, &writer_state) != 0) {
                                fprintf(stderr, "httpServe: failed to commit response head during auto-end\n");
                                auto_end_failed = 1;
                            }
                        }
                        if (!auto_end_failed && !taida_net3_is_bodyless_status(writer_state.pending_status)) {
                            taida_net_send_all(client_fd, "0\r\n\r\n", 5);
                        }
                        writer_state.state = NET3_STATE_ENDED;
                        if (auto_end_failed) {
                            // Force connection close
                            keep_alive = 0;
                        }
                    }
                }

                taida_release(request);
                taida_release(writer_token);
                taida_release(response);

                // NET4-1g: If body not fully read, do NOT return to keep-alive.
                int body_done = body_state.fully_read || (!is_chunked && content_length == 0);
                if (!body_done) keep_alive = 0;

                // NB5-24: Recover trailing bytes from body_state leftover.
                // When a pipelined client sends the next request in the same TCP
                // segment as the current body, those bytes end up in leftover beyond
                // the body data. Copy them back into the connection buffer so the
                // keep-alive loop can parse the next request from them.
                size_t trailing_len = 0;
                if (body_state.leftover && body_state.leftover_pos < body_state.leftover_len) {
                    trailing_len = body_state.leftover_len - body_state.leftover_pos;
                }
                if (trailing_len > 0 && keep_alive) {
                    if (trailing_len > buf_cap) {
                        buf_cap = trailing_len > 8192 ? trailing_len : 8192;
                        free(buf);
                        buf = (unsigned char*)TAIDA_MALLOC(buf_cap, "net_worker_buf");
                    }
                    memcpy(buf, body_state.leftover + body_state.leftover_pos, trailing_len);
                    total_read = trailing_len;
                } else {
                    total_read = 0;
                }

                // NB5-24: Skip the shared "Buffer advance" section — the 2-arg path
                // manages its own buffer state (total_read already set correctly).
                skip_buffer_advance = 1;

                if (leftover) free(leftover);
            } else {
                // ── v2/v3 1-arg handler path (unchanged eager body read) ──
                size_t body_start;
                size_t body_len;
                int64_t final_content_length;

                if (is_chunked) {
                    for (;;) {
                        int64_t check = taida_net_chunked_body_complete(buf, total_read, head_consumed);
                        if (check >= 0) break;
                        if (check == -2) {
                            const char *bad = "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                            taida_net_send_all(client_fd, bad, strlen(bad));
                            if (parse_result) taida_release(parse_result);
                            goto conn_done;
                        }
                        if (total_read >= NET_MAX_REQUEST_BUF) {
                            const char *bad = "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                            taida_net_send_all(client_fd, bad, strlen(bad));
                            if (parse_result) taida_release(parse_result);
                            goto conn_done;
                        }
                        if (total_read >= buf_cap) {
                            size_t new_cap = buf_cap * 2;
                            if (new_cap > NET_MAX_REQUEST_BUF) new_cap = NET_MAX_REQUEST_BUF;
                            TAIDA_REALLOC(buf, new_cap, "net_worker_chunked");
                            buf_cap = new_cap;
                        }
                        ssize_t n = taida_tls_recv(client_fd, buf + total_read, buf_cap - total_read);
                        if (n <= 0) {
                            const char *bad = "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                            taida_net_send_all(client_fd, bad, strlen(bad));
                            if (parse_result) taida_release(parse_result);
                            goto conn_done;
                        }
                        total_read += (size_t)n;
                    }
                    ChunkedCompactResult compact;
                    if (taida_net_chunked_in_place_compact(buf, head_consumed, &compact) < 0) {
                        const char *bad = "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                        taida_net_send_all(client_fd, bad, strlen(bad));
                        if (parse_result) taida_release(parse_result);
                        goto conn_done;
                    }
                    wire_consumed = head_consumed + compact.wire_consumed;
                    body_start = head_consumed;
                    body_len = compact.body_len;
                    final_content_length = (int64_t)compact.body_len;
                } else {
                    if (head_consumed + (size_t)content_length > NET_MAX_REQUEST_BUF) {
                        const char *too_large = "HTTP/1.1 413 Content Too Large\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                        taida_net_send_all(client_fd, too_large, strlen(too_large));
                        if (parse_result) taida_release(parse_result);
                        break;
                    }
                    size_t body_needed = head_consumed + (size_t)content_length;
                    while (total_read < body_needed && total_read < NET_MAX_REQUEST_BUF) {
                        if (total_read >= buf_cap) {
                            size_t new_cap = buf_cap * 2;
                            if (new_cap > NET_MAX_REQUEST_BUF) new_cap = NET_MAX_REQUEST_BUF;
                            TAIDA_REALLOC(buf, new_cap, "net_worker_body");
                            buf_cap = new_cap;
                        }
                        ssize_t n = taida_tls_recv(client_fd, buf + total_read, buf_cap - total_read);
                        if (n <= 0) break;
                        total_read += (size_t)n;
                    }
                    if (content_length > 0 && total_read < body_needed) {
                        const char *bad = "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                        taida_net_send_all(client_fd, bad, strlen(bad));
                        if (parse_result) taida_release(parse_result);
                        break;
                    }
                    wire_consumed = body_needed;
                    body_start = head_consumed;
                    body_len = (size_t)content_length;
                    final_content_length = content_length;
                }

                size_t raw_len = is_chunked ? (head_consumed + body_len) : wire_consumed;
                taida_val http_minor = 1;
                taida_val parsed_headers = 0;
                if (parse_inner != 0 && taida_is_buchi_pack(parse_inner)) {
                    taida_val ver = taida_pack_get(parse_inner, taida_str_hash((taida_val)"version"));
                    if (ver != 0 && taida_is_buchi_pack(ver)) {
                        http_minor = taida_pack_get(ver, taida_str_hash((taida_val)"minor"));
                    }
                    parsed_headers = taida_pack_get(parse_inner, taida_str_hash((taida_val)"headers"));
                }
                keep_alive = taida_net_determine_keep_alive(buf, raw_len, parsed_headers, http_minor);

                taida_val raw_bytes = taida_bytes_from_raw(buf, (taida_val)raw_len);
                taida_val request = taida_pack_new(13);
                taida_pack_set_hash(request, 0, taida_str_hash((taida_val)"raw"));
                taida_pack_set(request, 0, raw_bytes);
                taida_pack_set_tag(request, 0, TAIDA_TAG_PACK);

                if (parse_inner != 0 && taida_is_buchi_pack(parse_inner)) {
                    taida_val method_v = taida_pack_get(parse_inner, taida_str_hash((taida_val)"method"));
                    taida_pack_set_hash(request, 1, taida_str_hash((taida_val)"method"));
                    taida_pack_set(request, 1, method_v);
                    taida_pack_set_tag(request, 1, TAIDA_TAG_PACK);
                    if (method_v > 4096) taida_retain(method_v);
                    taida_val path_v = taida_pack_get(parse_inner, taida_str_hash((taida_val)"path"));
                    taida_pack_set_hash(request, 2, taida_str_hash((taida_val)"path"));
                    taida_pack_set(request, 2, path_v);
                    taida_pack_set_tag(request, 2, TAIDA_TAG_PACK);
                    if (path_v > 4096) taida_retain(path_v);
                    taida_val query_v = taida_pack_get(parse_inner, taida_str_hash((taida_val)"query"));
                    taida_pack_set_hash(request, 3, taida_str_hash((taida_val)"query"));
                    taida_pack_set(request, 3, query_v);
                    taida_pack_set_tag(request, 3, TAIDA_TAG_PACK);
                    if (query_v > 4096) taida_retain(query_v);
                    taida_val version_v = taida_pack_get(parse_inner, taida_str_hash((taida_val)"version"));
                    taida_pack_set_hash(request, 4, taida_str_hash((taida_val)"version"));
                    taida_pack_set(request, 4, version_v);
                    taida_pack_set_tag(request, 4, TAIDA_TAG_PACK);
                    if (version_v > 4096) taida_retain(version_v);
                    taida_val headers_v = taida_pack_get(parse_inner, taida_str_hash((taida_val)"headers"));
                    taida_pack_set_hash(request, 5, taida_str_hash((taida_val)"headers"));
                    taida_pack_set(request, 5, headers_v);
                    taida_pack_set_tag(request, 5, TAIDA_TAG_LIST);
                    if (headers_v > 4096) taida_retain(headers_v);
                } else {
                    taida_pack_set_hash(request, 1, taida_str_hash((taida_val)"method"));
                    taida_pack_set(request, 1, taida_net_make_span(0, 0));
                    taida_pack_set_tag(request, 1, TAIDA_TAG_PACK);
                    taida_pack_set_hash(request, 2, taida_str_hash((taida_val)"path"));
                    taida_pack_set(request, 2, taida_net_make_span(0, 0));
                    taida_pack_set_tag(request, 2, TAIDA_TAG_PACK);
                    taida_pack_set_hash(request, 3, taida_str_hash((taida_val)"query"));
                    taida_pack_set(request, 3, taida_net_make_span(0, 0));
                    taida_pack_set_tag(request, 3, TAIDA_TAG_PACK);
                    taida_val ver = taida_pack_new(2);
                    taida_pack_set_hash(ver, 0, taida_str_hash((taida_val)"major"));
                    taida_pack_set(ver, 0, 1);
                    taida_pack_set_hash(ver, 1, taida_str_hash((taida_val)"minor"));
                    taida_pack_set(ver, 1, 1);
                    taida_pack_set_hash(request, 4, taida_str_hash((taida_val)"version"));
                    taida_pack_set(request, 4, ver);
                    taida_pack_set_tag(request, 4, TAIDA_TAG_PACK);
                    taida_pack_set_hash(request, 5, taida_str_hash((taida_val)"headers"));
                    taida_pack_set(request, 5, taida_list_new());
                    taida_pack_set_tag(request, 5, TAIDA_TAG_LIST);
                }
                taida_pack_set_hash(request, 6, taida_str_hash((taida_val)"body"));
                taida_pack_set(request, 6, taida_net_make_span((taida_val)body_start, (taida_val)body_len));
                taida_pack_set_tag(request, 6, TAIDA_TAG_PACK);
                taida_pack_set_hash(request, 7, taida_str_hash((taida_val)"bodyOffset"));
                taida_pack_set(request, 7, (taida_val)body_start);
                taida_pack_set_hash(request, 8, taida_str_hash((taida_val)"contentLength"));
                taida_pack_set(request, 8, (taida_val)final_content_length);
                taida_pack_set_hash(request, 9, taida_str_hash((taida_val)"remoteHost"));
                taida_pack_set(request, 9, (taida_val)taida_str_new_copy(peer_host));
                taida_pack_set_tag(request, 9, TAIDA_TAG_STR);
                taida_pack_set_hash(request, 10, taida_str_hash((taida_val)"remotePort"));
                taida_pack_set(request, 10, (taida_val)peer_port);
                taida_pack_set_hash(request, 11, taida_str_hash((taida_val)"keepAlive"));
                taida_pack_set(request, 11, keep_alive ? 1 : 0);
                taida_pack_set_tag(request, 11, TAIDA_TAG_BOOL);
                taida_pack_set_hash(request, 12, taida_str_hash((taida_val)"chunked"));
                taida_pack_set(request, 12, is_chunked ? 1 : 0);
                taida_pack_set_tag(request, 12, TAIDA_TAG_BOOL);

                if (parse_result) { taida_release(parse_result); parse_result = 0; }

                // NB6-1: 1-arg handler — scatter-gather send (head+body separate).
                taida_val response = taida_invoke_callback1(pool->handler, request);
                if (taida_net_send_response_scatter(client_fd, response) != 0) {
                    const char *fallback = "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                    taida_net_send_all(client_fd, fallback, strlen(fallback));
                }
                taida_release(request);
                taida_release(response);
            }

            // request_count already reserved after head complete — check limit
            pthread_mutex_lock(&pool->mutex);
            int limit_hit = net_pool_requests_exhausted(pool);
            pthread_mutex_unlock(&pool->mutex);

            // Buffer advance: remove consumed bytes, keep any leftover.
            // NB5-24: Skip for 2-arg path — it manages its own buffer state.
            if (!skip_buffer_advance) {
                if (wire_consumed < total_read) {
                    memmove(buf, buf + wire_consumed, total_read - wire_consumed);
                    total_read -= wire_consumed;
                } else {
                    total_read = 0;
                }
            }

            // Close if not keep-alive or limit reached
            if (!keep_alive || limit_hit) break;
        }

    conn_done:
        // NET5-4a: TLS shutdown before closing fd.
        if (conn_ssl) {
            taida_tls_shutdown_free(conn_ssl);
            conn_ssl = NULL;
            tl_ssl = NULL;
        }
        close(client_fd);
        free(buf);
        buf = NULL;
        total_read = 0;
        buf_cap = 8192;

        // Re-allocate buffer for next connection
        // (will be done at top of next keep-alive loop iteration)

        // Decrement active connections and signal main thread
        pthread_mutex_lock(&pool->mutex);
        pool->active_connections--;
        pthread_cond_signal(&pool->cond_done);
        pthread_mutex_unlock(&pool->mutex);

        #undef NET_MAX_REQUEST_BUF
    }

    return NULL;
}

