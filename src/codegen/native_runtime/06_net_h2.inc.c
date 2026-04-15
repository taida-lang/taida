// ── Native HTTP/2 server (NET6-3a: h2 parity with Interpreter) ──────────────
//
// Reference: src/interpreter/net_h2.rs
// Design decisions:
//   - Blocking I/O (single-threaded per-connection, matching the interpreter model)
//   - One connection at a time (accept → serve → next)
//   - Stream multiplexing within a connection (serial handler dispatch)
//   - Connection-local buffers reused across frames
//   - No aggregate frame buffer; head and body are distinct
//   - ALPN "h2" is required (no silent h1 fallback)
//   - h2c (cleartext HTTP/2) is out of scope

// ── H2 constants (mirrors net_h2.rs) ──────────────────────────────────────

#define H2_CONNECTION_PREFACE "PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n"
#define H2_CONNECTION_PREFACE_LEN 24

#define H2_DEFAULT_INITIAL_WINDOW 65535
#define H2_DEFAULT_MAX_FRAME_SIZE 16384
#define H2_MAX_MAX_FRAME_SIZE     16777215
#define H2_DEFAULT_HEADER_TABLE_SIZE 4096
#define H2_DEFAULT_MAX_CONCURRENT_STREAMS 128
// RFC 9113 Section 6.9.1: flow-control window MUST NOT exceed 2^31-1
#define H2_MAX_FLOW_CONTROL_WINDOW ((int64_t)0x7FFFFFFF)
// Safety limits for HPACK bomb / memory exhaustion protection
#define H2_MAX_CONTINUATION_BUFFER_SIZE (128 * 1024)
#define H2_MAX_DECODED_HEADER_LIST_SIZE (64 * 1024)

// Frame types
#define H2_FRAME_DATA         0x0
#define H2_FRAME_HEADERS      0x1
#define H2_FRAME_PRIORITY     0x2
#define H2_FRAME_RST_STREAM   0x3
#define H2_FRAME_SETTINGS     0x4
#define H2_FRAME_PUSH_PROMISE 0x5
#define H2_FRAME_PING         0x6
#define H2_FRAME_GOAWAY       0x7
#define H2_FRAME_WINDOW_UPDATE 0x8
#define H2_FRAME_CONTINUATION 0x9

// Flags
#define H2_FLAG_END_STREAM  0x1
#define H2_FLAG_ACK         0x1
#define H2_FLAG_END_HEADERS 0x4
#define H2_FLAG_PADDED      0x8
#define H2_FLAG_PRIORITY    0x20

// Error codes
#define H2_ERROR_NO_ERROR          0x0
#define H2_ERROR_PROTOCOL_ERROR    0x1
#define H2_ERROR_INTERNAL_ERROR    0x2
#define H2_ERROR_FLOW_CONTROL_ERROR 0x3
#define H2_ERROR_FRAME_SIZE_ERROR  0x6
#define H2_ERROR_STREAM_CLOSED     0x5
#define H2_ERROR_COMPRESSION_ERROR 0x9

// Settings identifiers
#define H2_SETTINGS_HEADER_TABLE_SIZE      0x1
#define H2_SETTINGS_ENABLE_PUSH            0x2
#define H2_SETTINGS_MAX_CONCURRENT_STREAMS 0x3
#define H2_SETTINGS_INITIAL_WINDOW_SIZE    0x4
#define H2_SETTINGS_MAX_FRAME_SIZE         0x5
#define H2_SETTINGS_MAX_HEADER_LIST_SIZE   0x6

// ── H2 HPACK static table (RFC 7541 Appendix A) ───────────────────────────

typedef struct {
    const char *name;
    const char *value;
} H2HpackStaticEntry;

static const H2HpackStaticEntry H2_STATIC_TABLE[] = {
    { "", "" },                            // 0: unused
    { ":authority", "" },                  // 1
    { ":method", "GET" },                  // 2
    { ":method", "POST" },                 // 3
    { ":path", "/" },                      // 4
    { ":path", "/index.html" },            // 5
    { ":scheme", "http" },                 // 6
    { ":scheme", "https" },                // 7
    { ":status", "200" },                  // 8
    { ":status", "204" },                  // 9
    { ":status", "206" },                  // 10
    { ":status", "304" },                  // 11
    { ":status", "400" },                  // 12
    { ":status", "404" },                  // 13
    { ":status", "500" },                  // 14
    { "accept-charset", "" },              // 15
    { "accept-encoding", "gzip, deflate" },// 16
    { "accept-language", "" },             // 17
    { "accept-ranges", "" },               // 18
    { "accept", "" },                      // 19
    { "access-control-allow-origin", "" }, // 20
    { "age", "" },                         // 21
    { "allow", "" },                       // 22
    { "authorization", "" },               // 23
    { "cache-control", "" },               // 24
    { "content-disposition", "" },         // 25
    { "content-encoding", "" },            // 26
    { "content-language", "" },            // 27
    { "content-length", "" },              // 28
    { "content-location", "" },            // 29
    { "content-range", "" },               // 30
    { "content-type", "" },                // 31
    { "cookie", "" },                      // 32
    { "date", "" },                        // 33
    { "etag", "" },                        // 34
    { "expect", "" },                      // 35
    { "expires", "" },                     // 36
    { "from", "" },                        // 37
    { "host", "" },                        // 38
    { "if-match", "" },                    // 39
    { "if-modified-since", "" },           // 40
    { "if-none-match", "" },               // 41
    { "if-range", "" },                    // 42
    { "if-unmodified-since", "" },         // 43
    { "last-modified", "" },               // 44
    { "link", "" },                        // 45
    { "location", "" },                    // 46
    { "max-forwards", "" },                // 47
    { "proxy-authenticate", "" },          // 48
    { "proxy-authorization", "" },         // 49
    { "range", "" },                       // 50
    { "referer", "" },                     // 51
    { "refresh", "" },                     // 52
    { "retry-after", "" },                 // 53
    { "server", "" },                      // 54
    { "set-cookie", "" },                  // 55
    { "strict-transport-security", "" },   // 56
    { "transfer-encoding", "" },           // 57
    { "user-agent", "" },                  // 58
    { "vary", "" },                        // 59
    { "via", "" },                         // 60
    { "www-authenticate", "" },            // 61
};
#define H2_STATIC_TABLE_LEN (sizeof(H2_STATIC_TABLE) / sizeof(H2_STATIC_TABLE[0]))

// ── H2 HPACK dynamic table ─────────────────────────────────────────────────

typedef struct {
    char *name;
    char *value;
} H2HpackDynEntry;

typedef struct {
    H2HpackDynEntry *entries;  // Ring buffer (newest at index 0 semantics via head/len)
    int cap;                   // Total allocated slots
    int len;                   // Current count
    size_t current_size;       // Current byte size (name + value + 32 each)
    size_t max_size;           // Maximum allowed size
} H2HpackDynTable;

static void h2_dyntable_init(H2HpackDynTable *dt, size_t max_size) {
    dt->entries = NULL;
    dt->cap = 0;
    dt->len = 0;
    dt->current_size = 0;
    dt->max_size = max_size;
}

static void h2_dyntable_free(H2HpackDynTable *dt) {
    for (int i = 0; i < dt->len; i++) {
        free(dt->entries[i].name);
        free(dt->entries[i].value);
    }
    free(dt->entries);
    dt->entries = NULL;
    dt->len = 0;
    dt->cap = 0;
    dt->current_size = 0;
}

static size_t h2_entry_size(const char *name, const char *value) {
    return strlen(name) + strlen(value) + 32;
}

static void h2_dyntable_evict_to_fit(H2HpackDynTable *dt, size_t needed) {
    // NB6-33: Oldest entries are at the front (index 0). Evict from front.
    while (dt->len > 0 && dt->current_size + needed > dt->max_size) {
        dt->current_size -= h2_entry_size(dt->entries[0].name, dt->entries[0].value);
        free(dt->entries[0].name);
        free(dt->entries[0].value);
        // Shift remaining entries left by 1
        dt->len--;
        if (dt->len > 0) {
            memmove(&dt->entries[0], &dt->entries[1], (size_t)dt->len * sizeof(H2HpackDynEntry));
        }
    }
}

static void h2_dyntable_insert(H2HpackDynTable *dt, const char *name, const char *value) {
    size_t sz = h2_entry_size(name, value);
    h2_dyntable_evict_to_fit(dt, sz);
    if (sz > dt->max_size) return; // Entry too large even alone

    // Grow array if needed
    if (dt->len >= dt->cap) {
        int new_cap = dt->cap ? dt->cap * 2 : 8;
        H2HpackDynEntry *new_entries = (H2HpackDynEntry*)realloc(dt->entries,
            (size_t)new_cap * sizeof(H2HpackDynEntry));
        if (!new_entries) return;
        dt->entries = new_entries;
        dt->cap = new_cap;
    }

    // NB6-33: Append at end — O(1) instead of memmove O(n).
    // Newest entries are at the end (index len-1), oldest at front (index 0).
    // NB6-37: Check strdup return values to avoid segfault on OOM.
    char *dup_name = strdup(name);
    char *dup_value = strdup(value);
    if (!dup_name || !dup_value) {
        free(dup_name);
        free(dup_value);
        return;
    }
    dt->entries[dt->len].name = dup_name;
    dt->entries[dt->len].value = dup_value;
    dt->len++;
    dt->current_size += sz;
}

static void h2_dyntable_set_max_size(H2HpackDynTable *dt, size_t new_max) {
    dt->max_size = new_max;
    h2_dyntable_evict_to_fit(dt, 0);
}

// Get entry by 1-based combined index (static + dynamic).
// Returns 0 on success, -1 on out-of-range.
// NB6-33: Dynamic table is stored newest-at-end. HPACK index 0 = newest = entries[len-1].
static int h2_hpack_get_indexed(H2HpackDynTable *dt, size_t index,
                                 const char **name_out, const char **value_out) {
    if (index == 0) return -1;
    if (index < H2_STATIC_TABLE_LEN) {
        *name_out = H2_STATIC_TABLE[index].name;
        *value_out = H2_STATIC_TABLE[index].value;
        return 0;
    }
    size_t dyn_idx = index - H2_STATIC_TABLE_LEN;
    if ((int)dyn_idx >= dt->len) return -1;
    // Map HPACK dynamic index to array position: index 0 = newest = entries[len-1]
    int array_idx = dt->len - 1 - (int)dyn_idx;
    *name_out = dt->entries[array_idx].name;
    *value_out = dt->entries[array_idx].value;
    return 0;
}

static int h2_hpack_get_indexed_name(H2HpackDynTable *dt, size_t index, const char **name_out) {
    const char *v;
    return h2_hpack_get_indexed(dt, index, name_out, &v);
}

// ── H2 HPACK integer coding (RFC 7541 Section 5.1) ────────────────────────

// Decode HPACK integer with prefix_bits prefix.
// Returns bytes consumed, or -1 on error.
static int h2_hpack_decode_int(const unsigned char *data, size_t data_len,
                                uint8_t prefix_bits, size_t *value_out) {
    if (data_len == 0) return -1;
    uint8_t mask = (uint8_t)((1u << prefix_bits) - 1u);
    size_t value = data[0] & mask;
    int pos = 1;
    if (value < (size_t)mask) {
        *value_out = value;
        return pos;
    }
    // Multi-byte
    int shift = 0;
    while (pos < (int)data_len) {
        uint8_t byte = data[pos++];
        value += (size_t)(byte & 0x7F) << shift;
        shift += 7;
        if (!(byte & 0x80)) {
            *value_out = value;
            return pos;
        }
        if (shift > 28) return -1; // overflow guard
    }
    return -1; // truncated
}

// Encode HPACK integer into buf.  Returns bytes written.
static int h2_hpack_encode_int(unsigned char *buf, size_t buf_cap,
                                size_t value, uint8_t prefix_bits, uint8_t prefix_pattern) {
    uint8_t mask = (uint8_t)((1u << prefix_bits) - 1u);
    if (value < (size_t)mask) {
        if (buf_cap < 1) return -1;
        buf[0] = prefix_pattern | (uint8_t)value;
        return 1;
    }
    if (buf_cap < 1) return -1;
    buf[0] = prefix_pattern | mask;
    int pos = 1;
    size_t remaining = value - mask;
    while (remaining >= 128) {
        if (pos >= (int)buf_cap) return -1;
        buf[pos++] = (unsigned char)((remaining & 0x7F) | 0x80);
        remaining >>= 7;
    }
    if (pos >= (int)buf_cap) return -1;
    buf[pos++] = (unsigned char)remaining;
    return pos;
}

// ── H2 HPACK Huffman decode (RFC 7541 Appendix B) ─────────────────────────

// Minimal bit-by-bit Huffman decoder.
// The full table is in net_h2.rs; we duplicate the same data here.
typedef struct { uint8_t sym; uint32_t code; uint8_t bits; } H2HuffEntry;
static const H2HuffEntry H2_HUFFMAN_TABLE[] = {
    { 48, 0x00,  5},{ 49, 0x01,  5},{ 50, 0x02,  5},{ 97, 0x03,  5},
    { 99, 0x04,  5},{101, 0x05,  5},{105, 0x06,  5},{111, 0x07,  5},
    {115, 0x08,  5},{116, 0x09,  5},{ 32, 0x14,  6},{ 37, 0x15,  6},
    { 45, 0x16,  6},{ 46, 0x17,  6},{ 47, 0x18,  6},{ 51, 0x19,  6},
    { 52, 0x1a,  6},{ 53, 0x1b,  6},{ 54, 0x1c,  6},{ 55, 0x1d,  6},
    { 56, 0x1e,  6},{ 57, 0x1f,  6},{ 61, 0x20,  6},{ 65, 0x21,  6},
    { 95, 0x22,  6},{ 98, 0x23,  6},{100, 0x24,  6},{102, 0x25,  6},
    {103, 0x26,  6},{104, 0x27,  6},{108, 0x28,  6},{109, 0x29,  6},
    {110, 0x2a,  6},{112, 0x2b,  6},{114, 0x2c,  6},{117, 0x2d,  6},
    { 58, 0x5c,  7},{ 66, 0x5d,  7},{ 67, 0x5e,  7},{ 68, 0x5f,  7},
    { 69, 0x60,  7},{ 70, 0x61,  7},{ 71, 0x62,  7},{ 72, 0x63,  7},
    { 73, 0x64,  7},{ 74, 0x65,  7},{ 75, 0x66,  7},{ 76, 0x67,  7},
    { 77, 0x68,  7},{ 78, 0x69,  7},{ 79, 0x6a,  7},{ 80, 0x6b,  7},
    { 81, 0x6c,  7},{ 82, 0x6d,  7},{ 83, 0x6e,  7},{ 84, 0x6f,  7},
    { 85, 0x70,  7},{ 86, 0x71,  7},{ 87, 0x72,  7},{ 89, 0x73,  7},
    {106, 0x74,  7},{107, 0x75,  7},{113, 0x76,  7},{118, 0x77,  7},
    {119, 0x78,  7},{120, 0x79,  7},{121, 0x7a,  7},{122, 0x7b,  7},
    { 38, 0xf8,  8},{ 42, 0xf9,  8},{ 44, 0xfa,  8},{ 59, 0xfb,  8},
    { 88, 0xfc,  8},{ 90, 0xfd,  8},{ 33, 0x3f8,10},{ 34, 0x3f9,10},
    { 40, 0x3fa,10},{ 41, 0x3fb,10},{ 63, 0x3fc,10},{ 39, 0x7fa,11},
    { 43, 0x7fb,11},{124, 0x7fc,11},{ 35, 0xffa,12},{ 62, 0xffb,12},
    {  0, 0x1ff8,13},{ 36, 0x1ff9,13},{ 64, 0x1ffa,13},{ 91, 0x1ffb,13},
    { 93, 0x1ffc,13},{126, 0x1ffd,13},{ 94, 0x3ffc,14},{125, 0x3ffd,14},
    { 60, 0x7ffc,15},{ 96, 0x7ffd,15},{123, 0x7ffe,15},{ 92, 0x7fff0,19},
    {195, 0x7fff1,19},{208, 0x7fff2,19},{128, 0xfffe6,20},{130, 0xfffe7,20},
    {131, 0xfffe8,20},{162, 0xfffe9,20},{184, 0xfffea,20},{194, 0xfffeb,20},
    {224, 0xfffec,20},{226, 0xfffed,20},{153, 0x1fffdc,21},{161, 0x1fffdd,21},
    {167, 0x1fffde,21},{172, 0x1fffdf,21},{176, 0x1fffe0,21},{177, 0x1fffe1,21},
    {179, 0x1fffe2,21},{209, 0x1fffe3,21},{216, 0x1fffe4,21},{217, 0x1fffe5,21},
    {227, 0x1fffe6,21},{229, 0x1fffe7,21},{230, 0x1fffe8,21},{129, 0x3fffd2,22},
    {132, 0x3fffd3,22},{133, 0x3fffd4,22},{134, 0x3fffd5,22},{136, 0x3fffd6,22},
    {146, 0x3fffd7,22},{154, 0x3fffd8,22},{156, 0x3fffd9,22},{160, 0x3fffda,22},
    {163, 0x3fffdb,22},{164, 0x3fffdc,22},{169, 0x3fffdd,22},{170, 0x3fffde,22},
    {173, 0x3fffdf,22},{178, 0x3fffe0,22},{181, 0x3fffe1,22},{185, 0x3fffe2,22},
    {186, 0x3fffe3,22},{187, 0x3fffe4,22},{189, 0x3fffe5,22},{190, 0x3fffe6,22},
    {196, 0x3fffe7,22},{198, 0x3fffe8,22},{228, 0x3fffe9,22},{232, 0x3fffea,22},
    {233, 0x3fffeb,22},{  1, 0x7fffd8,23},{135, 0x7fffd9,23},{137, 0x7fffda,23},
    {138, 0x7fffdb,23},{139, 0x7fffdc,23},{140, 0x7fffdd,23},{141, 0x7fffde,23},
    {143, 0x7fffdf,23},{147, 0x7fffe0,23},{149, 0x7fffe1,23},{150, 0x7fffe2,23},
    {151, 0x7fffe3,23},{152, 0x7fffe4,23},{155, 0x7fffe5,23},{157, 0x7fffe6,23},
    {158, 0x7fffe7,23},{165, 0x7fffe8,23},{166, 0x7fffe9,23},{168, 0x7fffea,23},
    {174, 0x7fffeb,23},{175, 0x7fffec,23},{180, 0x7fffed,23},{182, 0x7fffee,23},
    {183, 0x7fffef,23},{188, 0x7ffff0,23},{191, 0x7ffff1,23},{197, 0x7ffff2,23},
    {231, 0x7ffff3,23},{239, 0x7ffff4,23},{  9, 0xffffea,24},{142, 0xffffeb,24},
    {144, 0xffffec,24},{145, 0xffffed,24},{148, 0xffffee,24},{159, 0xffffef,24},
    {171, 0xfffff0,24},{206, 0xfffff1,24},{215, 0xfffff2,24},{225, 0xfffff3,24},
    {236, 0xfffff4,24},{237, 0xfffff5,24},{199, 0x1ffffec,25},{207, 0x1ffffed,25},
    {234, 0x1ffffee,25},{235, 0x1ffffef,25},{192, 0x3ffffdc,26},{193, 0x3ffffdd,26},
    {200, 0x3ffffde,26},{201, 0x3ffffdf,26},{202, 0x3ffffe0,26},{205, 0x3ffffe1,26},
    {210, 0x3ffffe2,26},{213, 0x3ffffe3,26},{218, 0x3ffffe4,26},{219, 0x3ffffe5,26},
    {238, 0x3ffffe6,26},{240, 0x3ffffe7,26},{242, 0x3ffffe8,26},{243, 0x3ffffe9,26},
    {255, 0x3ffffea,26},{203, 0x7ffffd6,27},{204, 0x7ffffd7,27},{211, 0x7ffffd8,27},
    {212, 0x7ffffd9,27},{214, 0x7ffffda,27},{221, 0x7ffffdb,27},{222, 0x7ffffdc,27},
    {223, 0x7ffffdd,27},{241, 0x7ffffde,27},{244, 0x7ffffdf,27},{245, 0x7ffffe0,27},
    {246, 0x7ffffe1,27},{247, 0x7ffffe2,27},{248, 0x7ffffe3,27},{250, 0x7ffffe4,27},
    {251, 0x7ffffe5,27},{252, 0x7ffffe6,27},{253, 0x7ffffe7,27},{254, 0x7ffffe8,27},
    {  2, 0xfffffe2,28},{  3, 0xfffffe3,28},{  4, 0xfffffe4,28},{  5, 0xfffffe5,28},
    {  6, 0xfffffe6,28},{  7, 0xfffffe7,28},{  8, 0xfffffe8,28},{ 11, 0xfffffe9,28},
    { 12, 0xfffffea,28},{ 14, 0xfffffeb,28},{ 15, 0xfffffec,28},{ 16, 0xfffffed,28},
    { 17, 0xfffffee,28},{ 18, 0xfffffef,28},{ 19, 0xffffff0,28},{ 20, 0xffffff1,28},
    { 21, 0xffffff2,28},{ 23, 0xffffff3,28},{ 24, 0xffffff4,28},{ 25, 0xffffff5,28},
    { 26, 0xffffff6,28},{ 27, 0xffffff7,28},{ 28, 0xffffff8,28},{ 29, 0xffffff9,28},
    { 30, 0xffffffa,28},{ 31, 0xffffffb,28},{127, 0xffffffc,28},{220, 0xffffffd,28},
    {249, 0xffffffe,28},{ 10, 0x3ffffffc,30},{ 13, 0x3ffffffd,30},{ 22, 0x3ffffffe,30},
    /* NB7-75: RFC 7541 Section 5.2 — EOS (256) must be in table so decoder can reject it */
    {256, 0x3fffffff,30},
};
#define H2_HUFFMAN_TABLE_LEN (sizeof(H2_HUFFMAN_TABLE)/sizeof(H2_HUFFMAN_TABLE[0]))

// NB6-34: 8-bit prefix lookup table for fast Huffman decode.
// Entries with code length <= 8 are decoded in O(1). Longer codes fall back
// to a reduced linear scan (only entries with bits > 8).
typedef struct {
    uint8_t sym;
    uint8_t bits;  // 0 means no match at this prefix (need longer codes)
} H2HuffLookup;

static H2HuffLookup h2_huff_lut[256];
static int h2_huff_lut_initialized = 0;

// Build the 8-bit lookup table from the Huffman code table.
// Each 8-bit value maps to the symbol decoded by matching the MSBs.
static void h2_huff_build_lut(void) {
    if (h2_huff_lut_initialized) return;
    memset(h2_huff_lut, 0, sizeof(h2_huff_lut));
    for (size_t t = 0; t < H2_HUFFMAN_TABLE_LEN; t++) {
        uint8_t code_len = H2_HUFFMAN_TABLE[t].bits;
        if (code_len == 0 || code_len > 8) continue;
        // Shift code to fill 8-bit prefix, then fill all suffixes
        uint32_t code = H2_HUFFMAN_TABLE[t].code;
        int pad = 8 - code_len;
        uint32_t base = code << pad;
        uint32_t count = (uint32_t)1 << pad;
        for (uint32_t j = 0; j < count; j++) {
            uint32_t idx = base | j;
            if (idx < 256) {
                h2_huff_lut[idx].sym = H2_HUFFMAN_TABLE[t].sym;
                h2_huff_lut[idx].bits = code_len;
            }
        }
    }
    h2_huff_lut_initialized = 1;
}

// Decode a Huffman-encoded byte string into dst.
// Returns decoded byte count, or -1 on error.
static int h2_huffman_decode(const unsigned char *src, size_t src_len,
                              unsigned char *dst, size_t dst_cap) {
    h2_huff_build_lut();
    uint64_t bits = 0;
    uint8_t bits_left = 0;
    int out = 0;

    for (size_t i = 0; i < src_len; i++) {
        bits = (bits << 8) | src[i];
        bits_left += 8;

        while (bits_left >= 5) {
            // Fast path: try 8-bit LUT.
            // When bits_left >= 8, extract the top 8 bits directly.
            // When 5 <= bits_left < 8, left-shift to form an 8-bit prefix
            // and check that the matched code fits within bits_left.
            {
                uint8_t prefix;
                if (bits_left >= 8) {
                    prefix = (uint8_t)(bits >> (bits_left - 8));
                } else {
                    prefix = (uint8_t)(bits << (8 - bits_left));
                }
                H2HuffLookup *entry = &h2_huff_lut[prefix];
                if (entry->bits > 0 && entry->bits <= bits_left) {
                    /* NB7-75: RFC 7541 Section 5.2 — EOS symbol (256) forbidden */
                    if (entry->sym == 256) return -1;
                    if (out >= (int)dst_cap) return -1;
                    dst[out++] = entry->sym;
                    bits_left -= entry->bits;
                    bits &= bits_left ? (((uint64_t)1 << bits_left) - 1) : 0;
                    continue;
                }
            }
            // Slow path: linear scan for codes > 8 bits
            int found = 0;
            for (size_t t = 0; t < H2_HUFFMAN_TABLE_LEN; t++) {
                uint8_t code_len = H2_HUFFMAN_TABLE[t].bits;
                if (code_len <= 8) continue;  // Already handled by LUT
                if (bits_left < code_len) continue;
                uint8_t shift = bits_left - code_len;
                uint32_t candidate = (uint32_t)(bits >> shift);
                if (candidate == H2_HUFFMAN_TABLE[t].code) {
                    /* NB7-75: RFC 7541 Section 5.2 — EOS symbol (256) forbidden */
                    if (H2_HUFFMAN_TABLE[t].sym == 256) return -1;
                    if (out >= (int)dst_cap) return -1;
                    dst[out++] = H2_HUFFMAN_TABLE[t].sym;
                    bits_left -= code_len;
                    bits &= ((uint64_t)1 << bits_left) - 1;
                    found = 1;
                    break;
                }
            }
            if (!found) {
                if (bits_left < 30) break;
                return -1; // invalid
            }
        }
    }
    // Check padding: remaining bits must be 0-7 and all 1s.
    if (bits_left > 7) return -1;
    if (bits_left > 0) {
        uint64_t pad_mask = ((uint64_t)1 << bits_left) - 1;
        if ((bits & pad_mask) != pad_mask) return -1;
    }
    return out;
}

// ── H2 HPACK string coding ─────────────────────────────────────────────────

// Decode an HPACK string (length-prefixed, optionally Huffman).
// Writes null-terminated result into out_buf (up to out_cap-1 bytes).
// Returns total bytes consumed from data, or -1 on error.
static int h2_hpack_decode_string(const unsigned char *data, size_t data_len,
                                   char *out_buf, size_t out_cap) {
    if (data_len == 0) return -1;
    int huffman = (data[0] & 0x80) != 0;
    size_t str_len;
    int consumed = h2_hpack_decode_int(data, data_len, 7, &str_len);
    if (consumed < 0) return -1;
    if ((size_t)consumed + str_len > data_len) return -1;

    const unsigned char *raw = data + consumed;
    if (huffman) {
        int dec_len = h2_huffman_decode(raw, str_len, (unsigned char*)out_buf, out_cap - 1);
        if (dec_len < 0) return -1;
        out_buf[dec_len] = '\0';
    } else {
        if (str_len >= out_cap) return -1;
        memcpy(out_buf, raw, str_len);
        out_buf[str_len] = '\0';
    }
    return consumed + (int)str_len;
}

// Encode a raw (non-Huffman) HPACK string into buf.
// Returns bytes written, or -1 on overflow.
static int h2_hpack_encode_string(unsigned char *buf, size_t buf_cap, const char *s) {
    size_t slen = strlen(s);
    unsigned char int_buf[8];
    int int_sz = h2_hpack_encode_int(int_buf, sizeof(int_buf), slen, 7, 0x00);
    if (int_sz < 0 || (size_t)int_sz + slen > buf_cap) return -1;
    memcpy(buf, int_buf, (size_t)int_sz);
    memcpy(buf + int_sz, s, slen);
    return int_sz + (int)slen;
}

// ── H2 HPACK full header block decode/encode ──────────────────────────────

// NB6-29: Increased from 64 to 128 headers.
// NB6-30: Prevents premature COMPRESSION_ERROR for legitimate many-header requests.
#define H2_MAX_HEADERS 128
// NB6-29: Increased from 4096 to 16384 for value, 256 to 1024 for name.
// Brings Native closer to Interpreter's unlimited dynamic strings while keeping
// bounded memory. Interpreter still enforces MAX_DECODED_HEADER_LIST_SIZE (64KB).
#define H2_HEADER_NAME_SIZE 1024
#define H2_HEADER_BUF_SIZE 16384

typedef struct {
    char name[H2_HEADER_NAME_SIZE];
    char value[H2_HEADER_BUF_SIZE];
} H2Header;

// Decode an HPACK header block.
// Returns number of decoded headers, or -1 on error.
static int h2_hpack_decode_block(const unsigned char *data, size_t data_len,
                                  H2HpackDynTable *dyn,
                                  H2Header *headers, int max_headers) {
    int count = 0;
    size_t pos = 0;

    while (pos < data_len) {
        if (count >= max_headers) return -1;
        uint8_t byte = data[pos];

        if (byte & 0x80) {
            // Indexed header field (Section 6.1)
            size_t index;
            int consumed = h2_hpack_decode_int(data + pos, data_len - pos, 7, &index);
            if (consumed < 0) return -1;
            pos += (size_t)consumed;
            const char *n, *v;
            if (h2_hpack_get_indexed(dyn, index, &n, &v) < 0) return -1;
            snprintf(headers[count].name, sizeof(headers[count].name), "%s", n);
            snprintf(headers[count].value, sizeof(headers[count].value), "%s", v);
            count++;
        } else if (byte & 0x40) {
            // Literal with incremental indexing (Section 6.2.1)
            size_t index;
            int consumed = h2_hpack_decode_int(data + pos, data_len - pos, 6, &index);
            if (consumed < 0) return -1;
            pos += (size_t)consumed;
            char name_buf[H2_HEADER_NAME_SIZE], value_buf[H2_HEADER_BUF_SIZE];
            if (index == 0) {
                int ns = h2_hpack_decode_string(data + pos, data_len - pos, name_buf, sizeof(name_buf));
                if (ns < 0) return -1;
                pos += (size_t)ns;
            } else {
                const char *n;
                if (h2_hpack_get_indexed_name(dyn, index, &n) < 0) return -1;
                snprintf(name_buf, sizeof(name_buf), "%s", n);
            }
            int vs = h2_hpack_decode_string(data + pos, data_len - pos, value_buf, sizeof(value_buf));
            if (vs < 0) return -1;
            pos += (size_t)vs;
            h2_dyntable_insert(dyn, name_buf, value_buf);
            snprintf(headers[count].name, sizeof(headers[count].name), "%s", name_buf);
            snprintf(headers[count].value, sizeof(headers[count].value), "%s", value_buf);
            count++;
        } else if (byte & 0x20) {
            // Dynamic table size update (Section 6.3)
            size_t new_size;
            int consumed = h2_hpack_decode_int(data + pos, data_len - pos, 5, &new_size);
            if (consumed < 0) return -1;
            pos += (size_t)consumed;
            h2_dyntable_set_max_size(dyn, new_size);
        } else {
            // Literal without/never indexing (Sections 6.2.2 / 6.2.3)
            uint8_t prefix = (byte & 0x10) ? 4 : 4;
            size_t index;
            int consumed = h2_hpack_decode_int(data + pos, data_len - pos, prefix, &index);
            if (consumed < 0) return -1;
            pos += (size_t)consumed;
            char name_buf[H2_HEADER_NAME_SIZE], value_buf[H2_HEADER_BUF_SIZE];
            if (index == 0) {
                int ns = h2_hpack_decode_string(data + pos, data_len - pos, name_buf, sizeof(name_buf));
                if (ns < 0) return -1;
                pos += (size_t)ns;
            } else {
                const char *n;
                if (h2_hpack_get_indexed_name(dyn, index, &n) < 0) return -1;
                snprintf(name_buf, sizeof(name_buf), "%s", n);
            }
            int vs = h2_hpack_decode_string(data + pos, data_len - pos, value_buf, sizeof(value_buf));
            if (vs < 0) return -1;
            pos += (size_t)vs;
            snprintf(headers[count].name, sizeof(headers[count].name), "%s", name_buf);
            snprintf(headers[count].value, sizeof(headers[count].value), "%s", value_buf);
            count++;
        }
    }
    return count;
}

// Encode a list of headers into an HPACK block in buf.
// Returns bytes written, or -1 on overflow/error.
static int h2_hpack_encode_block(unsigned char *buf, size_t buf_cap,
                                  H2HpackDynTable *enc_dyn,
                                  const H2Header *headers, int count) {
    int pos = 0;
    for (int i = 0; i < count; i++) {
        const char *name = headers[i].name;
        const char *value = headers[i].value;

        // Try static table exact match
        int exact_idx = -1;
        int name_idx = -1;
        for (int s = 1; s < (int)H2_STATIC_TABLE_LEN; s++) {
            if (strcmp(H2_STATIC_TABLE[s].name, name) == 0) {
                if (name_idx < 0) name_idx = s;
                if (H2_STATIC_TABLE[s].value[0] != '\0' &&
                    strcmp(H2_STATIC_TABLE[s].value, value) == 0) {
                    exact_idx = s;
                    break;
                }
            }
        }

        if (exact_idx > 0) {
            // Indexed header field
            unsigned char tmp[8];
            int n = h2_hpack_encode_int(tmp, sizeof(tmp), (size_t)exact_idx, 7, 0x80);
            if (n < 0 || pos + n > (int)buf_cap) return -1;
            memcpy(buf + pos, tmp, (size_t)n);
            pos += n;
        } else if (name_idx > 0) {
            // Literal with incremental indexing, indexed name
            unsigned char tmp[8];
            int n = h2_hpack_encode_int(tmp, sizeof(tmp), (size_t)name_idx, 6, 0x40);
            if (n < 0 || pos + n > (int)buf_cap) return -1;
            memcpy(buf + pos, tmp, (size_t)n);
            pos += n;
            int vs = h2_hpack_encode_string(buf + pos, buf_cap - (size_t)pos, value);
            if (vs < 0) return -1;
            pos += vs;
            h2_dyntable_insert(enc_dyn, name, value);
        } else {
            // Literal with incremental indexing, new name
            if (pos >= (int)buf_cap) return -1;
            buf[pos++] = 0x40;
            int ns = h2_hpack_encode_string(buf + pos, buf_cap - (size_t)pos, name);
            if (ns < 0) return -1;
            pos += ns;
            int vs = h2_hpack_encode_string(buf + pos, buf_cap - (size_t)pos, value);
            if (vs < 0) return -1;
            pos += vs;
            h2_dyntable_insert(enc_dyn, name, value);
        }
    }
    return pos;
}

// ── H2 stream state ────────────────────────────────────────────────────────

#define H2_STREAM_IDLE              0
#define H2_STREAM_HALF_CLOSED_REMOTE 1
#define H2_STREAM_CLOSED            2

typedef struct {
    uint32_t stream_id;
    int state;
    H2Header *request_headers;
    int request_header_count;
    unsigned char *request_body;
    size_t request_body_len;
    size_t request_body_cap;
    int64_t send_window;
    int64_t recv_window;
} H2Stream;

// Simple stream table (small fixed-size array for the blocking serial model)
#define H2_MAX_STREAMS 256

typedef struct {
    H2Stream streams[H2_MAX_STREAMS];
    int stream_count;
    H2HpackDynTable decoder_dyn;
    H2HpackDynTable encoder_dyn;
    int64_t conn_send_window;
    int64_t conn_recv_window;
    uint32_t peer_max_frame_size;
    uint32_t peer_initial_window_size;
    uint32_t local_max_frame_size;
    uint32_t last_peer_stream_id;
    int goaway_sent;
    // CONTINUATION state
    unsigned char *continuation_buf;
    size_t continuation_len;
    size_t continuation_cap;
    uint32_t continuation_stream_id;
    uint8_t continuation_flags;
} H2Conn;

// NB6-41: Search from end — most recent streams are at higher indices,
// and the hot-path frame loop typically references the latest stream.
static H2Stream *h2_conn_find_stream(H2Conn *conn, uint32_t stream_id) {
    for (int i = conn->stream_count - 1; i >= 0; i--) {
        if (conn->streams[i].stream_id == stream_id) return &conn->streams[i];
    }
    return NULL;
}

static H2Stream *h2_conn_new_stream(H2Conn *conn, uint32_t stream_id) {
    if (conn->stream_count >= H2_MAX_STREAMS) return NULL;
    H2Stream *s = &conn->streams[conn->stream_count++];
    memset(s, 0, sizeof(*s));
    s->stream_id = stream_id;
    s->state = H2_STREAM_IDLE;
    s->request_headers = NULL;
    s->request_header_count = 0;
    s->request_body = NULL;
    s->request_body_len = 0;
    s->request_body_cap = 0;
    s->send_window = (int64_t)conn->peer_initial_window_size;
    s->recv_window = H2_DEFAULT_INITIAL_WINDOW;
    return s;
}

static void h2_stream_free(H2Stream *s) {
    free(s->request_headers);
    s->request_headers = NULL;
    free(s->request_body);
    s->request_body = NULL;
}

static void h2_conn_remove_closed_streams(H2Conn *conn) {
    int new_count = 0;
    for (int i = 0; i < conn->stream_count; i++) {
        if (conn->streams[i].state != H2_STREAM_CLOSED) {
            if (i != new_count) conn->streams[new_count] = conn->streams[i];
            new_count++;
        } else {
            h2_stream_free(&conn->streams[i]);
        }
    }
    conn->stream_count = new_count;
}

static void h2_conn_init(H2Conn *conn) {
    memset(conn, 0, sizeof(*conn));
    h2_dyntable_init(&conn->decoder_dyn, H2_DEFAULT_HEADER_TABLE_SIZE);
    h2_dyntable_init(&conn->encoder_dyn, H2_DEFAULT_HEADER_TABLE_SIZE);
    conn->conn_send_window = H2_DEFAULT_INITIAL_WINDOW;
    conn->conn_recv_window = H2_DEFAULT_INITIAL_WINDOW;
    conn->peer_max_frame_size = H2_DEFAULT_MAX_FRAME_SIZE;
    conn->peer_initial_window_size = H2_DEFAULT_INITIAL_WINDOW;
    conn->local_max_frame_size = H2_DEFAULT_MAX_FRAME_SIZE;
    conn->goaway_sent = 0;
}

static void h2_conn_free(H2Conn *conn) {
    for (int i = 0; i < conn->stream_count; i++) h2_stream_free(&conn->streams[i]);
    conn->stream_count = 0;
    h2_dyntable_free(&conn->decoder_dyn);
    h2_dyntable_free(&conn->encoder_dyn);
    free(conn->continuation_buf);
    conn->continuation_buf = NULL;
    conn->continuation_len = 0;
    conn->continuation_cap = 0;
}

// ── H2 frame I/O helpers ───────────────────────────────────────────────────

// Read exactly n bytes. Returns n on success, 0 on clean EOF, -1 on error.
static int h2_read_exact(int fd, unsigned char *buf, size_t n) {
    size_t pos = 0;
    while (pos < n) {
        ssize_t r = taida_tls_recv(fd, buf + pos, n - pos);
        if (r <= 0) return (r == 0 && pos == 0) ? 0 : -1;
        pos += (size_t)r;
    }
    return (int)n;
}

// Write all bytes. Returns 0 on success, -1 on error.
// taida_tls_send_all returns 0 on success, -1 on error — pass through directly.
static int h2_write_all(int fd, const unsigned char *buf, size_t n) {
    return taida_tls_send_all(fd, buf, n);
}

// Write a single H2 frame (9-byte header + payload).
// frame_type, flags, stream_id, payload/payload_len.
static int h2_write_frame(int fd, uint8_t frame_type, uint8_t flags,
                           uint32_t stream_id, const unsigned char *payload, uint32_t payload_len) {
    unsigned char header[9];
    header[0] = (payload_len >> 16) & 0xFF;
    header[1] = (payload_len >> 8) & 0xFF;
    header[2] = payload_len & 0xFF;
    header[3] = frame_type;
    header[4] = flags;
    header[5] = (stream_id >> 24) & 0x7F;
    header[6] = (stream_id >> 16) & 0xFF;
    header[7] = (stream_id >> 8) & 0xFF;
    header[8] = stream_id & 0xFF;
    if (h2_write_all(fd, header, 9) < 0) return -1;
    if (payload_len > 0 && h2_write_all(fd, payload, (size_t)payload_len) < 0) return -1;
    return 0;
}

// Validate that decoded header list does not exceed safety limit.
// Returns 0 on success, -1 if headers are too large.
// RFC 9113 Section 6.5.2: size = sum of (name_len + value_len + 32) per entry.
static int h2_validate_header_list_size(const H2Header *headers, int count) {
    size_t total = 0;
    for (int i = 0; i < count; i++) {
        total += strlen(headers[i].name) + strlen(headers[i].value) + 32;
        if (total > H2_MAX_DECODED_HEADER_LIST_SIZE) return -1;
    }
    return 0;
}

// Read one frame. Returns 1 on success, 0 on clean close, -1 on error/protocol violation.
// On success, *payload_out is malloc'd (caller must free), *payload_len_out is set.
static int h2_read_frame(int fd, uint32_t max_frame_size,
                          uint8_t *type_out, uint8_t *flags_out, uint32_t *stream_id_out,
                          unsigned char **payload_out, uint32_t *payload_len_out) {
    unsigned char header[9];
    int r = h2_read_exact(fd, header, 9);
    if (r == 0) return 0;
    if (r < 0) return -1;

    uint32_t len = ((uint32_t)header[0] << 16) | ((uint32_t)header[1] << 8) | header[2];
    *type_out = header[3];
    *flags_out = header[4];
    *stream_id_out = (((uint32_t)(header[5] & 0x7F)) << 24) |
                     ((uint32_t)header[6] << 16) |
                     ((uint32_t)header[7] << 8)  |
                      (uint32_t)header[8];
    *payload_len_out = len;

    if (len > max_frame_size) return -2; // FRAME_SIZE_ERROR

    if (len > 0) {
        *payload_out = (unsigned char*)TAIDA_MALLOC((size_t)len, "h2_frame_payload");
        if (!*payload_out) return -1;
        if (h2_read_exact(fd, *payload_out, (size_t)len) != (int)len) {
            free(*payload_out);
            *payload_out = NULL;
            return -1;
        }
    } else {
        *payload_out = NULL;
    }
    return 1;
}

// Send GOAWAY frame (connection-level error/graceful shutdown).
static int h2_send_goaway(int fd, uint32_t last_stream_id,
                           uint32_t error_code, const char *debug_data) {
    size_t debug_len = debug_data ? strlen(debug_data) : 0;
    size_t payload_len = 8 + debug_len;
    unsigned char *payload = (unsigned char*)TAIDA_MALLOC(payload_len, "h2_goaway_payload");
    if (!payload) return -1;
    payload[0] = (last_stream_id >> 24) & 0x7F;
    payload[1] = (last_stream_id >> 16) & 0xFF;
    payload[2] = (last_stream_id >> 8) & 0xFF;
    payload[3] = last_stream_id & 0xFF;
    payload[4] = (error_code >> 24) & 0xFF;
    payload[5] = (error_code >> 16) & 0xFF;
    payload[6] = (error_code >> 8) & 0xFF;
    payload[7] = error_code & 0xFF;
    if (debug_len > 0) memcpy(payload + 8, debug_data, debug_len);
    int rc = h2_write_frame(fd, H2_FRAME_GOAWAY, 0, 0, payload, (uint32_t)payload_len);
    free(payload);
    return rc;
}

// Send RST_STREAM frame.
static int h2_send_rst_stream(int fd, uint32_t stream_id, uint32_t error_code) {
    unsigned char payload[4];
    payload[0] = (error_code >> 24) & 0xFF;
    payload[1] = (error_code >> 16) & 0xFF;
    payload[2] = (error_code >> 8) & 0xFF;
    payload[3] = error_code & 0xFF;
    return h2_write_frame(fd, H2_FRAME_RST_STREAM, 0, stream_id, payload, 4);
}

// Send SETTINGS frame with server defaults.
static int h2_send_server_settings(int fd, uint32_t max_frame_size, uint32_t max_concurrent_streams) {
    unsigned char payload[24]; // 4 settings * 6 bytes each
    int pos = 0;
    // MAX_CONCURRENT_STREAMS
    payload[pos++] = 0x00; payload[pos++] = 0x03;
    payload[pos++] = (max_concurrent_streams >> 24) & 0xFF;
    payload[pos++] = (max_concurrent_streams >> 16) & 0xFF;
    payload[pos++] = (max_concurrent_streams >> 8) & 0xFF;
    payload[pos++] = max_concurrent_streams & 0xFF;
    // INITIAL_WINDOW_SIZE
    payload[pos++] = 0x00; payload[pos++] = 0x04;
    payload[pos++] = 0x00; payload[pos++] = 0x00;
    payload[pos++] = 0xFF; payload[pos++] = 0xFF;
    // MAX_FRAME_SIZE
    payload[pos++] = 0x00; payload[pos++] = 0x05;
    payload[pos++] = (max_frame_size >> 24) & 0xFF;
    payload[pos++] = (max_frame_size >> 16) & 0xFF;
    payload[pos++] = (max_frame_size >> 8) & 0xFF;
    payload[pos++] = max_frame_size & 0xFF;
    // ENABLE_PUSH = 0
    payload[pos++] = 0x00; payload[pos++] = 0x02;
    payload[pos++] = 0x00; payload[pos++] = 0x00; payload[pos++] = 0x00; payload[pos++] = 0x00;
    return h2_write_frame(fd, H2_FRAME_SETTINGS, 0, 0, payload, (uint32_t)pos);
}

// Send SETTINGS ACK.
static int h2_send_settings_ack(int fd) {
    return h2_write_frame(fd, H2_FRAME_SETTINGS, H2_FLAG_ACK, 0, NULL, 0);
}

// Send WINDOW_UPDATE frame.
static int h2_send_window_update(int fd, uint32_t stream_id, uint32_t increment) {
    if (increment == 0 || increment > 0x7FFFFFFF) return -1;
    unsigned char payload[4];
    payload[0] = (increment >> 24) & 0x7F;
    payload[1] = (increment >> 16) & 0xFF;
    payload[2] = (increment >> 8) & 0xFF;
    payload[3] = increment & 0xFF;
    return h2_write_frame(fd, H2_FRAME_WINDOW_UPDATE, 0, stream_id, payload, 4);
}

// Send PING ACK.
static int h2_send_ping_ack(int fd, const unsigned char *opaque, uint32_t opaque_len) {
    return h2_write_frame(fd, H2_FRAME_PING, H2_FLAG_ACK, 0, opaque, opaque_len);
}

// ── H2 response send helpers ──────────────────────────────────────────────

// Send response HEADERS + optional CONTINUATION if the HPACK block is large.
// HPACK encodes ":status" + provided headers into resp_hdr_buf.
// peer_max_frame_size controls frame splitting.
// Returns 0 on success, -1 on error.
static int h2_send_response_headers(int fd, H2HpackDynTable *enc_dyn,
                                     uint32_t stream_id, int status_code,
                                     const H2Header *extra_headers, int extra_count,
                                     int end_stream, uint32_t peer_max_frame_size) {
    // Build header list
    H2Header all_headers[H2_MAX_HEADERS];
    int count = 0;
    // :status pseudo-header first
    snprintf(all_headers[0].name, sizeof(all_headers[0].name), ":status");
    snprintf(all_headers[0].value, sizeof(all_headers[0].value), "%d", status_code);
    count = 1;
    for (int i = 0; i < extra_count && count < H2_MAX_HEADERS; i++) {
        // Lowercase header names (HTTP/2 requires lowercase)
        size_t nlen = strlen(extra_headers[i].name);
        if (nlen >= sizeof(all_headers[count].name)) nlen = sizeof(all_headers[count].name) - 1;
        for (size_t j = 0; j < nlen; j++) {
            all_headers[count].name[j] = (char)tolower((unsigned char)extra_headers[i].name[j]);
        }
        all_headers[count].name[nlen] = '\0';
        snprintf(all_headers[count].value, sizeof(all_headers[count].value), "%s", extra_headers[i].value);
        count++;
    }

    // NB6-24: Use 8KB stack buffer + heap fallback instead of fixed 64KB malloc.
    // Most response headers are small (< 1KB); 8KB covers typical cases without heap.
    unsigned char hdr_stack[8192];
    size_t hdr_buf_cap = sizeof(hdr_stack);
    unsigned char *hdr_buf = hdr_stack;

    int enc_len = h2_hpack_encode_block(hdr_buf, hdr_buf_cap, enc_dyn,
                                         (const H2Header*)all_headers, count);
    // If stack buffer was too small, retry with heap
    if (enc_len < 0 && hdr_buf == hdr_stack) {
        hdr_buf_cap = 65536;
        hdr_buf = (unsigned char*)TAIDA_MALLOC(hdr_buf_cap, "h2_hdr_block_fallback");
        if (!hdr_buf) return -1;
        enc_len = h2_hpack_encode_block(hdr_buf, hdr_buf_cap, enc_dyn,
                                         (const H2Header*)all_headers, count);
    }
    if (enc_len < 0) { if (hdr_buf != hdr_stack) free(hdr_buf); return -1; }

    uint32_t max_sz = peer_max_frame_size;
    if ((uint32_t)enc_len <= max_sz) {
        // Single HEADERS frame
        uint8_t flags = H2_FLAG_END_HEADERS;
        if (end_stream) flags |= H2_FLAG_END_STREAM;
        int rc = h2_write_frame(fd, H2_FRAME_HEADERS, flags, stream_id, hdr_buf, (uint32_t)enc_len);
        if (hdr_buf != hdr_stack) free(hdr_buf);
        return rc;
    }

    // Split: HEADERS (no END_HEADERS) + CONTINUATION*
    uint8_t flags = 0;
    if (end_stream) flags |= H2_FLAG_END_STREAM;
    if (h2_write_frame(fd, H2_FRAME_HEADERS, flags, stream_id, hdr_buf, max_sz) < 0) {
        if (hdr_buf != hdr_stack) free(hdr_buf); return -1;
    }
    uint32_t offset = max_sz;
    while (offset < (uint32_t)enc_len) {
        uint32_t chunk = (uint32_t)enc_len - offset;
        if (chunk > max_sz) chunk = max_sz;
        int is_last = (offset + chunk >= (uint32_t)enc_len);
        uint8_t cont_flags = is_last ? H2_FLAG_END_HEADERS : 0;
        if (h2_write_frame(fd, H2_FRAME_CONTINUATION, cont_flags, stream_id,
                           hdr_buf + offset, chunk) < 0) {
            if (hdr_buf != hdr_stack) free(hdr_buf); return -1;
        }
        offset += chunk;
    }
    if (hdr_buf != hdr_stack) free(hdr_buf);
    return 0;
}

// Send response DATA frames respecting flow control windows.
// Returns bytes sent, or -1 on error/window exhaustion.
static int64_t h2_send_response_data(int fd, uint32_t stream_id,
                                      const unsigned char *data, size_t data_len,
                                      int end_stream,
                                      uint32_t max_frame_size,
                                      int64_t *conn_send_window,
                                      int64_t *stream_send_window) {
    if (data_len == 0) {
        if (end_stream) h2_write_frame(fd, H2_FRAME_DATA, H2_FLAG_END_STREAM, stream_id, NULL, 0);
        return 0;
    }

    int64_t sent = 0;
    while ((size_t)sent < data_len) {
        size_t remaining = data_len - (size_t)sent;
        size_t frame_limit = (size_t)max_frame_size;
        size_t conn_limit = (*conn_send_window > 0) ? (size_t)*conn_send_window : 0;
        size_t stream_limit = (*stream_send_window > 0) ? (size_t)*stream_send_window : 0;
        size_t chunk = remaining;
        if (chunk > frame_limit) chunk = frame_limit;
        if (chunk > conn_limit) chunk = conn_limit;
        if (chunk > stream_limit) chunk = stream_limit;
        if (chunk == 0) return -1; // window exhausted

        int is_last = ((size_t)sent + chunk >= data_len);
        uint8_t flags = (is_last && end_stream) ? H2_FLAG_END_STREAM : 0;
        if (h2_write_frame(fd, H2_FRAME_DATA, flags, stream_id,
                           data + sent, (uint32_t)chunk) < 0) return -1;
        *conn_send_window -= (int64_t)chunk;
        *stream_send_window -= (int64_t)chunk;
        sent += (int64_t)chunk;
    }
    return sent;
}

// ── H2 frame processing ────────────────────────────────────────────────────

// Process a received SETTINGS frame payload.
static int h2_process_settings(H2Conn *conn, const unsigned char *payload, uint32_t len) {
    if (len % 6 != 0) return -1; // FRAME_SIZE_ERROR
    for (uint32_t i = 0; i + 6 <= len; i += 6) {
        uint16_t id = ((uint16_t)payload[i] << 8) | payload[i+1];
        uint32_t value = ((uint32_t)payload[i+2] << 24) | ((uint32_t)payload[i+3] << 16) |
                         ((uint32_t)payload[i+4] << 8) | payload[i+5];
        switch (id) {
            case H2_SETTINGS_HEADER_TABLE_SIZE:
                h2_dyntable_set_max_size(&conn->encoder_dyn, (size_t)value);
                break;
            case H2_SETTINGS_ENABLE_PUSH:
                if (value > 1) return -1;
                break;
            case H2_SETTINGS_MAX_CONCURRENT_STREAMS:
                // We note it but don't enforce for the blocking serial model
                break;
            case H2_SETTINGS_INITIAL_WINDOW_SIZE:
                if (value > 0x7FFFFFFF) return -1;
                {
                    int64_t delta = (int64_t)value - (int64_t)conn->peer_initial_window_size;
                    conn->peer_initial_window_size = value;
                    for (int s = 0; s < conn->stream_count; s++) {
                        conn->streams[s].send_window += delta;
                    }
                }
                break;
            case H2_SETTINGS_MAX_FRAME_SIZE:
                if (value < H2_DEFAULT_MAX_FRAME_SIZE || value > H2_MAX_MAX_FRAME_SIZE) return -1;
                conn->peer_max_frame_size = value;
                break;
            case H2_SETTINGS_MAX_HEADER_LIST_SIZE:
                break;
            default:
                break; // Unknown settings ignored
        }
    }
    return 0;
}

// ── H2 request extraction from decoded pseudo-headers ─────────────────────

// error_reason values for H2RequestFields (0 = no error)
#define H2_REQ_ERR_NONE            0
#define H2_REQ_ERR_ORDERING        1
#define H2_REQ_ERR_UNKNOWN_PSEUDO  2
#define H2_REQ_ERR_MISSING_PSEUDO  3

typedef struct {
    char method[16];
    char path[2048];
    char authority[256];
    H2Header *regular_headers;
    int regular_count;
    int ok;
    int error_reason;
} H2RequestFields;

// error_reason values for duplicate pseudo-headers
#define H2_REQ_ERR_DUPLICATE_PSEUDO 4
// error_reason values for empty pseudo-header values
#define H2_REQ_ERR_EMPTY_PSEUDO     5

static void h2_extract_request_fields(const H2Header *headers, int count, H2RequestFields *out) {
    memset(out, 0, sizeof(*out));
    out->regular_headers = NULL;
    out->regular_count = 0;
    out->ok = 0;
    out->error_reason = H2_REQ_ERR_NONE;

    char scheme[16] = "";
    int saw_regular = 0;
    int saw_method = 0, saw_path = 0, saw_authority = 0, saw_scheme = 0;
    H2Header *regs = (H2Header*)TAIDA_MALLOC(sizeof(H2Header) * (size_t)(count + 1), "h2_regular_headers");
    if (!regs) return;
    int reg_count = 0;

    for (int i = 0; i < count; i++) {
        if (headers[i].name[0] == ':') {
            if (saw_regular) {
                out->error_reason = H2_REQ_ERR_ORDERING;
                free(regs);
                return; // ordering violation
            }
            if (strcmp(headers[i].name, ":method") == 0) {
                // RFC 9113 Section 8.3.1: each pseudo-header MUST NOT appear more than once
                if (saw_method) {
                    out->error_reason = H2_REQ_ERR_DUPLICATE_PSEUDO;
                    free(regs);
                    return;
                }
                saw_method = 1;
                snprintf(out->method, sizeof(out->method), "%s", headers[i].value);
            } else if (strcmp(headers[i].name, ":path") == 0) {
                if (saw_path) {
                    out->error_reason = H2_REQ_ERR_DUPLICATE_PSEUDO;
                    free(regs);
                    return;
                }
                saw_path = 1;
                snprintf(out->path, sizeof(out->path), "%s", headers[i].value);
            } else if (strcmp(headers[i].name, ":authority") == 0) {
                if (saw_authority) {
                    out->error_reason = H2_REQ_ERR_DUPLICATE_PSEUDO;
                    free(regs);
                    return;
                }
                saw_authority = 1;
                snprintf(out->authority, sizeof(out->authority), "%s", headers[i].value);
            } else if (strcmp(headers[i].name, ":scheme") == 0) {
                if (saw_scheme) {
                    out->error_reason = H2_REQ_ERR_DUPLICATE_PSEUDO;
                    free(regs);
                    return;
                }
                saw_scheme = 1;
                snprintf(scheme, sizeof(scheme), "%s", headers[i].value);
            } else {
                // Unknown pseudo-header: reject as PROTOCOL_ERROR
                // (matches Interpreter: H2Error::Stream with ERROR_PROTOCOL_ERROR)
                out->error_reason = H2_REQ_ERR_UNKNOWN_PSEUDO;
                free(regs);
                return;
            }
        } else {
            saw_regular = 1;
            if (reg_count < count) {
                regs[reg_count++] = headers[i];
            }
        }
    }

    if (out->method[0] == '\0' || out->path[0] == '\0' || scheme[0] == '\0') {
        out->error_reason = H2_REQ_ERR_MISSING_PSEUDO;
        free(regs);
        return; // missing required pseudo-headers
    }
    out->regular_headers = regs;
    out->regular_count = reg_count;
    out->ok = 1;
}

// ── H2 response extraction from taida_val ─────────────────────────────────
// Mirrors extract_response_fields() in net_eval.rs.

typedef struct {
    int status;
    H2Header *headers;
    int header_count;
    unsigned char *body;
    size_t body_len;
    int ok;
} H2ResponseFields;

static void h2_extract_response_fields(taida_val response, H2ResponseFields *out) {
    memset(out, 0, sizeof(*out));
    out->status = 500;
    out->ok = 0;

    if (!TAIDA_IS_PACK(response)) return;

    // status
    taida_val status_hash = taida_str_hash((taida_val)"status");
    taida_val status_val = taida_pack_get(response, status_hash);
    if (status_val > 0 && status_val < 1000) {
        out->status = (int)status_val;
    } else {
        out->status = 500;
    }

    // headers: @[@(name: Str, value: Str)]
    taida_val hdrs_hash = taida_str_hash((taida_val)"headers");
    taida_val hdrs_val = taida_pack_get(response, hdrs_hash);
    int header_cap = 32;
    out->headers = (H2Header*)TAIDA_MALLOC(sizeof(H2Header) * (size_t)header_cap, "h2_resp_headers");
    if (!out->headers) return;
    out->header_count = 0;

    if (TAIDA_IS_LIST(hdrs_val)) {
        int64_t list_len = (int64_t)taida_list_length(hdrs_val);
        for (int64_t j = 0; j < list_len && out->header_count < header_cap; j++) {
            taida_val entry = taida_list_get(hdrs_val, (taida_val)j);
            if (!TAIDA_IS_PACK(entry)) continue;
            taida_val name_h = taida_str_hash((taida_val)"name");
            taida_val val_h  = taida_str_hash((taida_val)"value");
            taida_val n = taida_pack_get(entry, name_h);
            taida_val v = taida_pack_get(entry, val_h);
            if (!n || n <= 4096 || !v || v <= 4096) continue;
            snprintf(out->headers[out->header_count].name,
                     sizeof(out->headers[out->header_count].name), "%s", (const char*)n);
            snprintf(out->headers[out->header_count].value,
                     sizeof(out->headers[out->header_count].value), "%s", (const char*)v);
            out->header_count++;
        }
    }

    // body
    taida_val body_hash = taida_str_hash((taida_val)"body");
    taida_val body_val = taida_pack_get(response, body_hash);
    out->body = NULL;
    out->body_len = 0;

    if (body_val && body_val > 4096) {
        // Check if it's Bytes
        taida_val body_tag = taida_pack_get_field_tag(response, body_hash);
        if (body_tag == TAIDA_TAG_UNKNOWN) {
            body_tag = taida_runtime_detect_tag(body_val);
        }
        if (body_tag == TAIDA_TAG_STR) {
            const char *body_str = (const char*)body_val;
            size_t blen = strlen(body_str);
            out->body = (unsigned char*)TAIDA_MALLOC(blen + 1, "h2_resp_body");
            if (out->body) { memcpy(out->body, body_str, blen); out->body_len = blen; }
        } else if (TAIDA_IS_BYTES(body_val)) {
            // Bytes value: header[0]=magic, header[1]=len, then raw bytes inline
            int64_t blen = (int64_t)taida_bytes_len(body_val);
            if (blen > 0) {
                out->body = (unsigned char*)TAIDA_MALLOC((size_t)blen, "h2_resp_body_bytes");
                if (out->body) {
                    // Bytes layout: [magic|refcount, len, b0, b1, ...]
                    taida_val *bdata = (taida_val*)body_val;
                    for (int64_t bi = 0; bi < blen; bi++) {
                        out->body[bi] = (unsigned char)(bdata[2 + bi] & 0xFF);
                    }
                    out->body_len = (size_t)blen;
                }
            }
        }
    }
    out->ok = 1;
}

static void h2_response_fields_free(H2ResponseFields *r) {
    free(r->headers);
    r->headers = NULL;
    free(r->body);
    r->body = NULL;
}

// ── H2 serve one connection ────────────────────────────────────────────────
//
// Processes one HTTP/2 connection: reads frames, dispatches requests,
// sends responses. Returns after the connection closes or max_requests is reached.

typedef struct {
    taida_val handler;
    int handler_arity;
    int64_t *request_count;
    int64_t max_requests;
    char peer_host[64];
    int peer_port;
} H2ServeCtx;

// Call the Taida handler with the request pack and return the response value.
// Uses taida_invoke_callback1 — same calling convention as the h1 1-arg path.
static taida_val h2_dispatch_request(H2ServeCtx *ctx, taida_val request_pack) {
    return taida_invoke_callback1(ctx->handler, request_pack);
}

// Build a taida_val BuchiPack representing the HTTP/2 request.
// This mirrors the Interpreter's request pack in serve_h2().
static taida_val h2_build_request_pack(H2RequestFields *fields,
                                        const unsigned char *body, size_t body_len,
                                        const char *peer_host, int peer_port) {
    // Header list @[@(name: Str, value: Str)]
    taida_val hdr_list = taida_list_new();
    for (int i = 0; i < fields->regular_count; i++) {
        taida_val entry = taida_pack_new(2);
        taida_pack_set_hash(entry, 0, taida_str_hash((taida_val)"name"));
        taida_pack_set(entry, 0, (taida_val)taida_str_new_copy(fields->regular_headers[i].name));
        taida_pack_set_tag(entry, 0, TAIDA_TAG_STR);
        taida_pack_set_hash(entry, 1, taida_str_hash((taida_val)"value"));
        taida_pack_set(entry, 1, (taida_val)taida_str_new_copy(fields->regular_headers[i].value));
        taida_pack_set_tag(entry, 1, TAIDA_TAG_STR);
        hdr_list = taida_list_append(hdr_list, entry);
    }
    // :authority as host header
    if (fields->authority[0] != '\0') {
        taida_val entry = taida_pack_new(2);
        taida_pack_set_hash(entry, 0, taida_str_hash((taida_val)"name"));
        taida_pack_set(entry, 0, (taida_val)taida_str_new_copy("host"));
        taida_pack_set_tag(entry, 0, TAIDA_TAG_STR);
        taida_pack_set_hash(entry, 1, taida_str_hash((taida_val)"value"));
        taida_pack_set(entry, 1, (taida_val)taida_str_new_copy(fields->authority));
        taida_pack_set_tag(entry, 1, TAIDA_TAG_STR);
        hdr_list = taida_list_append(hdr_list, entry);
    }

    // Split path and query
    char path_part[2048], query_part[2048];
    const char *qmark = strchr(fields->path, '?');
    if (qmark) {
        size_t plen = (size_t)(qmark - fields->path);
        if (plen >= sizeof(path_part)) plen = sizeof(path_part) - 1;
        memcpy(path_part, fields->path, plen);
        path_part[plen] = '\0';
        snprintf(query_part, sizeof(query_part), "%s", qmark + 1);
    } else {
        snprintf(path_part, sizeof(path_part), "%s", fields->path);
        query_part[0] = '\0';
    }

    // NB6-26: Build proper Bytes (not List) for raw body — matches Interpreter's Value::Bytes(body)
    taida_val raw_bytes = taida_bytes_from_raw(body, (taida_val)body_len);

    // version pack @(major: 2, minor: 0)
    taida_val version_pack = taida_pack_new(2);
    taida_pack_set_hash(version_pack, 0, taida_str_hash((taida_val)"major"));
    taida_pack_set(version_pack, 0, (taida_val)2);
    taida_pack_set_tag(version_pack, 0, TAIDA_TAG_INT);
    taida_pack_set_hash(version_pack, 1, taida_str_hash((taida_val)"minor"));
    taida_pack_set(version_pack, 1, (taida_val)0);
    taida_pack_set_tag(version_pack, 1, TAIDA_TAG_INT);

    // NB6-28: Request pack: 14 fields (was 13 — missing "chunked")
    // Matches Interpreter's 14-field request pack.
    taida_val req = taida_pack_new(14);
    int f = 0;
    #define SET_FIELD(nm, val, tag) do { \
        taida_pack_set_hash(req, f, taida_str_hash((taida_val)(nm))); \
        taida_pack_set(req, f, (val)); \
        taida_pack_set_tag(req, f, (tag)); \
        f++; \
    } while(0)

    SET_FIELD("method",      (taida_val)taida_str_new_copy(fields->method), TAIDA_TAG_STR);
    SET_FIELD("path",        (taida_val)taida_str_new_copy(path_part),       TAIDA_TAG_STR);
    SET_FIELD("query",       (taida_val)taida_str_new_copy(query_part),      TAIDA_TAG_STR);
    SET_FIELD("version",     version_pack,                                 TAIDA_TAG_PACK);
    SET_FIELD("headers",     hdr_list,                                     TAIDA_TAG_LIST);
    // NB6-26: Use TAIDA_TAG_PACK for Bytes (consistent with h1 path — Bytes use PACK tag in Native)
    SET_FIELD("body",        raw_bytes,                                    TAIDA_TAG_PACK);
    SET_FIELD("bodyOffset",  (taida_val)0,                                 TAIDA_TAG_INT);
    SET_FIELD("contentLength",(taida_val)(int64_t)body_len,                TAIDA_TAG_INT);
    // NB6-27: Retain raw_bytes before setting as second field to prevent double-free
    taida_retain(raw_bytes);
    SET_FIELD("raw",         raw_bytes,                                    TAIDA_TAG_PACK);
    SET_FIELD("remoteHost",  (taida_val)taida_str_new_copy(peer_host),       TAIDA_TAG_STR);
    SET_FIELD("remotePort",  (taida_val)(int64_t)peer_port,                TAIDA_TAG_INT);
    SET_FIELD("keepAlive",   (taida_val)1,                                 TAIDA_TAG_BOOL);
    // NB6-28: Add missing "chunked" field (HTTP/2 never uses chunked TE)
    SET_FIELD("chunked",     (taida_val)0,                                 TAIDA_TAG_BOOL);
    SET_FIELD("protocol",    (taida_val)taida_str_new_copy("h2"),            TAIDA_TAG_STR);
    #undef SET_FIELD
    return req;
}

// Append data to the CONTINUATION buffer (resizing as needed).
static int h2_continuation_append(H2Conn *conn, const unsigned char *data, uint32_t len) {
    if (len == 0) return 0;
    // Safety limit: prevent HPACK bomb / memory exhaustion
    if (conn->continuation_len + (size_t)len > H2_MAX_CONTINUATION_BUFFER_SIZE) return -1;
    if (conn->continuation_len + len > conn->continuation_cap) {
        size_t new_cap = conn->continuation_cap ? conn->continuation_cap * 2 : 4096;
        while (new_cap < conn->continuation_len + len) new_cap *= 2;
        if (new_cap > H2_MAX_CONTINUATION_BUFFER_SIZE) new_cap = H2_MAX_CONTINUATION_BUFFER_SIZE;
        unsigned char *nb = (unsigned char*)realloc(conn->continuation_buf, new_cap);
        if (!nb) return -1;
        conn->continuation_buf = nb;
        conn->continuation_cap = new_cap;
    }
    memcpy(conn->continuation_buf + conn->continuation_len, data, len);
    conn->continuation_len += len;
    return 0;
}

// ── taida_net_h2_serve_connection ─────────────────────────────────────────
// Serve one HTTP/2 connection on file descriptor `client_fd`.
// Returns after connection closes or max_requests reached.
// conn_send_window_ptr and stream_send_window_ptr are temporarily per-call.
static void taida_net_h2_serve_connection(int client_fd, H2ServeCtx *ctx) {
    // NB6-40: Heap-allocate H2Conn (~18KB) to avoid deep-stack overflow risk.
    H2Conn *connp = (H2Conn*)TAIDA_MALLOC(sizeof(H2Conn), "h2_conn");
    if (!connp) return;
    #define conn (*connp)
    h2_conn_init(&conn);

    // Validate connection preface
    {
        unsigned char preface[H2_CONNECTION_PREFACE_LEN];
        if (h2_read_exact(client_fd, preface, H2_CONNECTION_PREFACE_LEN) != H2_CONNECTION_PREFACE_LEN) {
            goto h2_conn_done;
        }
        if (memcmp(preface, H2_CONNECTION_PREFACE, H2_CONNECTION_PREFACE_LEN) != 0) {
            h2_send_goaway(client_fd, 0, H2_ERROR_PROTOCOL_ERROR, "invalid connection preface");
            goto h2_conn_done;
        }
    }

    // Send server SETTINGS
    if (h2_send_server_settings(client_fd, H2_DEFAULT_MAX_FRAME_SIZE,
                                 H2_DEFAULT_MAX_CONCURRENT_STREAMS) < 0) {
        goto h2_conn_done;
    }

    // Connection frame loop
    {
        int settings_ack_pending = 0;

        for (;;) {
            if (ctx->max_requests > 0 && *ctx->request_count >= ctx->max_requests) break;

            uint8_t frame_type, frame_flags;
            uint32_t frame_stream_id, payload_len;
            unsigned char *payload = NULL;

            int fr = h2_read_frame(client_fd, conn.local_max_frame_size,
                                    &frame_type, &frame_flags, &frame_stream_id,
                                    &payload, &payload_len);
            if (fr == 0) break; // clean close
            if (fr == -2) {
                // FRAME_SIZE_ERROR
                h2_send_goaway(client_fd, conn.last_peer_stream_id,
                               H2_ERROR_FRAME_SIZE_ERROR, "frame too large");
                conn.goaway_sent = 1;
                break;
            }
            if (fr < 0) break;

            // RFC 9113: during CONTINUATION sequence only CONTINUATION is allowed
            if (conn.continuation_stream_id != 0 && frame_type != H2_FRAME_CONTINUATION) {
                free(payload);
                h2_send_goaway(client_fd, conn.last_peer_stream_id,
                               H2_ERROR_PROTOCOL_ERROR, "expected CONTINUATION");
                conn.goaway_sent = 1;
                break;
            }

            // Accumulate SETTINGS ACK / PING tracking
            int is_ping_ack_needed = 0;
            unsigned char ping_data[8];
            if (frame_type == H2_FRAME_SETTINGS && !(frame_flags & H2_FLAG_ACK)) {
                settings_ack_pending = 1;
            }
            if (frame_type == H2_FRAME_PING && !(frame_flags & H2_FLAG_ACK) && payload_len == 8) {
                is_ping_ack_needed = 1;
                memcpy(ping_data, payload, 8);
            }

            // Dispatch by frame type
            int protocol_error = 0;
            int completed_stream_id = 0; // Non-zero if a request is ready

            switch (frame_type) {
                case H2_FRAME_SETTINGS: {
                    if (frame_stream_id != 0) { protocol_error = 1; break; }
                    if (frame_flags & H2_FLAG_ACK) {
                        if (payload_len != 0) { protocol_error = 1; break; }
                        break;
                    }
                    if (h2_process_settings(&conn, payload, payload_len) < 0) {
                        protocol_error = 1;
                    }
                    break;
                }

                case H2_FRAME_HEADERS: {
                    if (frame_stream_id == 0) { protocol_error = 1; break; }
                    if (frame_stream_id % 2 == 0) { protocol_error = 1; break; }
                    if (frame_stream_id <= conn.last_peer_stream_id) { protocol_error = 1; break; }
                    conn.last_peer_stream_id = frame_stream_id;

                    // Strip padding
                    uint32_t offset = 0, pad_len = 0;
                    if (frame_flags & H2_FLAG_PADDED) {
                        if (payload_len == 0) { protocol_error = 1; break; }
                        pad_len = payload[0];
                        offset = 1;
                    }
                    if (frame_flags & H2_FLAG_PRIORITY) offset += 5;
                    if (offset + pad_len > payload_len) { protocol_error = 1; break; }

                    const unsigned char *hdr_block = payload + offset;
                    uint32_t hdr_block_len = payload_len - offset - pad_len;

                    int end_headers = (frame_flags & H2_FLAG_END_HEADERS) != 0;
                    int end_stream  = (frame_flags & H2_FLAG_END_STREAM)  != 0;

                    // Create stream slot
                    H2Stream *s = h2_conn_new_stream(&conn, frame_stream_id);
                    if (!s) { protocol_error = 1; break; }

                    if (!end_headers) {
                        // Start CONTINUATION sequence
                        conn.continuation_stream_id = frame_stream_id;
                        conn.continuation_flags = frame_flags;
                        conn.continuation_len = 0;
                        if (h2_continuation_append(&conn, hdr_block, hdr_block_len) < 0) {
                            protocol_error = 1;
                        }
                        break;
                    }

                    // END_HEADERS: decode now
                    H2Header *hdrs = (H2Header*)TAIDA_MALLOC(sizeof(H2Header) * H2_MAX_HEADERS, "h2_headers");
                    if (!hdrs) { protocol_error = 1; break; }
                    int hdr_count = h2_hpack_decode_block(hdr_block, hdr_block_len,
                                                           &conn.decoder_dyn, hdrs, H2_MAX_HEADERS);
                    if (hdr_count < 0) {
                        free(hdrs);
                        h2_send_goaway(client_fd, conn.last_peer_stream_id,
                                       H2_ERROR_COMPRESSION_ERROR, "HPACK decode error");
                        conn.goaway_sent = 1;
                        free(payload);
                        goto h2_conn_done;
                    }
                    // Safety: enforce header list size limit (HPACK bomb protection)
                    if (h2_validate_header_list_size(hdrs, hdr_count) < 0) {
                        free(hdrs);
                        h2_send_goaway(client_fd, conn.last_peer_stream_id,
                                       H2_ERROR_INTERNAL_ERROR, "decoded header list too large");
                        conn.goaway_sent = 1;
                        free(payload);
                        goto h2_conn_done;
                    }
                    s->request_headers = hdrs;
                    s->request_header_count = hdr_count;
                    s->state = H2_STREAM_HALF_CLOSED_REMOTE;

                    if (end_stream) {
                        completed_stream_id = (int)frame_stream_id;
                    }
                    break;
                }

                case H2_FRAME_DATA: {
                    if (frame_stream_id == 0) { protocol_error = 1; break; }
                    H2Stream *s = h2_conn_find_stream(&conn, frame_stream_id);
                    if (!s) { h2_send_rst_stream(client_fd, frame_stream_id, H2_ERROR_STREAM_CLOSED); break; }

                    // Strip padding
                    uint32_t offset = 0, pad_len = 0;
                    if (frame_flags & H2_FLAG_PADDED) {
                        if (payload_len == 0) { protocol_error = 1; break; }
                        pad_len = payload[0];
                        offset = 1;
                    }
                    if (offset + pad_len > payload_len) { protocol_error = 1; break; }

                    int64_t data_len = (int64_t)(payload_len); // includes padding in window
                    // Flow control enforcement
                    if (data_len > conn.conn_recv_window) {
                        h2_send_goaway(client_fd, conn.last_peer_stream_id,
                                       H2_ERROR_FLOW_CONTROL_ERROR, "connection recv window exceeded");
                        conn.goaway_sent = 1;
                        free(payload);
                        goto h2_conn_done;
                    }
                    if (data_len > s->recv_window) {
                        // Stream-level violation: RST_STREAM + close stream + continue
                        // (matches Interpreter: H2Error::Stream → send_rst_stream → continue)
                        h2_send_rst_stream(client_fd, frame_stream_id, H2_ERROR_FLOW_CONTROL_ERROR);
                        s->state = H2_STREAM_CLOSED;
                        h2_conn_remove_closed_streams(&conn);
                        free(payload);
                        continue;
                    }
                    conn.conn_recv_window -= data_len;
                    s->recv_window -= data_len;

                    const unsigned char *data = payload + offset;
                    uint32_t data_bytes = payload_len - offset - pad_len;
                    // Accumulate body
                    if (s->request_body_len + data_bytes > s->request_body_cap) {
                        size_t new_cap = s->request_body_cap ? s->request_body_cap * 2 : 4096;
                        while (new_cap < s->request_body_len + data_bytes) new_cap *= 2;
                        unsigned char *nb = (unsigned char*)realloc(s->request_body, new_cap);
                        if (!nb) { protocol_error = 1; break; }
                        s->request_body = nb;
                        s->request_body_cap = new_cap;
                    }
                    memcpy(s->request_body + s->request_body_len, data, data_bytes);
                    s->request_body_len += data_bytes;

                    if (frame_flags & H2_FLAG_END_STREAM) {
                        completed_stream_id = (int)frame_stream_id;
                    }
                    break;
                }

                case H2_FRAME_WINDOW_UPDATE: {
                    if (payload_len != 4) { protocol_error = 1; break; }
                    uint32_t increment = (((uint32_t)(payload[0] & 0x7F)) << 24) |
                                         ((uint32_t)payload[1] << 16) |
                                         ((uint32_t)payload[2] << 8)  |
                                          (uint32_t)payload[3];
                    if (increment == 0) { protocol_error = 1; break; }
                    if (frame_stream_id == 0) {
                        // RFC 9113 Section 6.9.1: window MUST NOT exceed 2^31-1
                        int64_t new_window = conn.conn_send_window + (int64_t)increment;
                        if (new_window > H2_MAX_FLOW_CONTROL_WINDOW) {
                            free(payload);
                            h2_send_goaway(client_fd, conn.last_peer_stream_id,
                                           H2_ERROR_FLOW_CONTROL_ERROR,
                                           "WINDOW_UPDATE would overflow connection window");
                            conn.goaway_sent = 1;
                            goto h2_conn_done;
                        }
                        conn.conn_send_window = new_window;
                    } else {
                        H2Stream *s = h2_conn_find_stream(&conn, frame_stream_id);
                        if (s) {
                            int64_t new_window = s->send_window + (int64_t)increment;
                            if (new_window > H2_MAX_FLOW_CONTROL_WINDOW) {
                                h2_send_rst_stream(client_fd, frame_stream_id,
                                                   H2_ERROR_FLOW_CONTROL_ERROR);
                                s->state = H2_STREAM_CLOSED;
                            } else {
                                s->send_window = new_window;
                            }
                        }
                    }
                    break;
                }

                case H2_FRAME_PING: {
                    if (frame_stream_id != 0) { protocol_error = 1; break; }
                    if (payload_len != 8) { protocol_error = 1; break; }
                    // ACK handled below
                    break;
                }

                case H2_FRAME_GOAWAY:
                    // Client is shutting down
                    free(payload);
                    goto h2_conn_done;

                case H2_FRAME_RST_STREAM: {
                    if (frame_stream_id == 0) { protocol_error = 1; break; }
                    // NB6-31: RFC 9113 Section 6.4 — RST_STREAM payload MUST be exactly 4 bytes
                    if (payload_len != 4) { protocol_error = 1; break; }
                    H2Stream *s = h2_conn_find_stream(&conn, frame_stream_id);
                    if (s) s->state = H2_STREAM_CLOSED;
                    break;
                }

                case H2_FRAME_PRIORITY: {
                    if (payload_len != 5) { protocol_error = 1; break; }
                    break; // advisory, ignored
                }

                case H2_FRAME_PUSH_PROMISE: {
                    // Client sending PUSH_PROMISE is a protocol error
                    protocol_error = 1;
                    break;
                }

                case H2_FRAME_CONTINUATION: {
                    if (conn.continuation_stream_id == 0) { protocol_error = 1; break; }
                    if (frame_stream_id != conn.continuation_stream_id) { protocol_error = 1; break; }

                    if (h2_continuation_append(&conn, payload, payload_len) < 0) {
                        protocol_error = 1; break;
                    }

                    int end_headers = (frame_flags & H2_FLAG_END_HEADERS) != 0;
                    if (!end_headers) break; // more CONTINUATION expected

                    // END_HEADERS: decode complete header block
                    uint32_t sid = conn.continuation_stream_id;
                    uint8_t orig_flags = conn.continuation_flags;
                    int end_stream = (orig_flags & H2_FLAG_END_STREAM) != 0;

                    H2Stream *s = h2_conn_find_stream(&conn, sid);
                    if (!s) {
                        // Create if not found (shouldn't happen for valid flow)
                        s = h2_conn_new_stream(&conn, sid);
                        if (!s) { protocol_error = 1; break; }
                    }

                    H2Header *hdrs = (H2Header*)TAIDA_MALLOC(sizeof(H2Header) * H2_MAX_HEADERS, "h2_cont_headers");
                    if (!hdrs) { protocol_error = 1; break; }
                    int hdr_count = h2_hpack_decode_block(conn.continuation_buf,
                                                           conn.continuation_len,
                                                           &conn.decoder_dyn, hdrs, H2_MAX_HEADERS);
                    conn.continuation_stream_id = 0;
                    conn.continuation_flags = 0;
                    conn.continuation_len = 0;

                    if (hdr_count < 0) {
                        free(hdrs);
                        h2_send_goaway(client_fd, conn.last_peer_stream_id,
                                       H2_ERROR_COMPRESSION_ERROR, "HPACK decode error in CONTINUATION");
                        conn.goaway_sent = 1;
                        free(payload);
                        goto h2_conn_done;
                    }
                    // Safety: enforce header list size limit (HPACK bomb protection)
                    if (h2_validate_header_list_size(hdrs, hdr_count) < 0) {
                        free(hdrs);
                        h2_send_goaway(client_fd, conn.last_peer_stream_id,
                                       H2_ERROR_INTERNAL_ERROR, "decoded header list too large in CONTINUATION");
                        conn.goaway_sent = 1;
                        free(payload);
                        goto h2_conn_done;
                    }
                    free(s->request_headers);
                    s->request_headers = hdrs;
                    s->request_header_count = hdr_count;
                    s->state = H2_STREAM_HALF_CLOSED_REMOTE;

                    if (end_stream) completed_stream_id = (int)sid;
                    break;
                }

                default:
                    break; // Unknown frame types ignored (RFC 9113 Section 4.1)
            }

            free(payload);

            if (protocol_error) {
                h2_send_goaway(client_fd, conn.last_peer_stream_id,
                               H2_ERROR_PROTOCOL_ERROR, "protocol error");
                conn.goaway_sent = 1;
                goto h2_conn_done;
            }

            // Send SETTINGS ACK if we processed a SETTINGS frame
            if (settings_ack_pending) {
                if (h2_send_settings_ack(client_fd) < 0) goto h2_conn_done;
                settings_ack_pending = 0;
            }

            // Send PING ACK if needed
            if (is_ping_ack_needed) {
                h2_send_ping_ack(client_fd, ping_data, 8);
            }

            // Dispatch completed request
            if (completed_stream_id > 0) {
                H2Stream *s = h2_conn_find_stream(&conn, (uint32_t)completed_stream_id);
                if (!s) continue;

                // Replenish receive window
                if (s->request_body_len > 0) {
                    uint32_t inc = (uint32_t)s->request_body_len;
                    h2_send_window_update(client_fd, 0, inc);
                    h2_send_window_update(client_fd, (uint32_t)completed_stream_id, inc);
                    conn.conn_recv_window += inc;
                    s->recv_window += inc;
                }

                // Extract request fields
                H2RequestFields req_fields;
                h2_extract_request_fields(s->request_headers, s->request_header_count, &req_fields);

                if (!req_fields.ok) {
                    h2_send_rst_stream(client_fd, (uint32_t)completed_stream_id, H2_ERROR_PROTOCOL_ERROR);
                    s->state = H2_STREAM_CLOSED;
                    h2_conn_remove_closed_streams(&conn);
                    continue;
                }

                // Build request pack and call handler
                taida_val req_pack = h2_build_request_pack(
                    &req_fields,
                    s->request_body, s->request_body_len,
                    ctx->peer_host, ctx->peer_port
                );
                free(req_fields.regular_headers);

                taida_val response = h2_dispatch_request(ctx, req_pack);
                (*ctx->request_count)++;

                // Extract and send response
                H2ResponseFields resp;
                h2_extract_response_fields(response, &resp);

                int no_body = (resp.status >= 100 && resp.status < 200) ||
                              resp.status == 204 || resp.status == 205 || resp.status == 304;
                int has_body = resp.ok && resp.body && resp.body_len > 0 && !no_body;

                if (!has_body) {
                    h2_send_response_headers(
                        client_fd, &conn.encoder_dyn,
                        (uint32_t)completed_stream_id, resp.status,
                        resp.headers, resp.header_count,
                        1 /*end_stream*/, conn.peer_max_frame_size
                    );
                } else {
                    // Add content-length if not present
                    int has_cl = 0;
                    for (int hi = 0; hi < resp.header_count; hi++) {
                        if (strcasecmp(resp.headers[hi].name, "content-length") == 0) {
                            has_cl = 1; break;
                        }
                    }
                    H2Header *all_hdrs = resp.headers;
                    int all_count = resp.header_count;
                    H2Header cl_hdr;
                    if (!has_cl) {
                        // Allocate extended header array
                        all_hdrs = (H2Header*)TAIDA_MALLOC(sizeof(H2Header) * (size_t)(resp.header_count + 1), "h2_resp_hdrs_cl");
                        if (all_hdrs) {
                            memcpy(all_hdrs, resp.headers, sizeof(H2Header) * (size_t)resp.header_count);
                            snprintf(cl_hdr.name, sizeof(cl_hdr.name), "content-length");
                            snprintf(cl_hdr.value, sizeof(cl_hdr.value), "%zu", resp.body_len);
                            all_hdrs[resp.header_count] = cl_hdr;
                            all_count = resp.header_count + 1;
                        } else {
                            all_hdrs = resp.headers;
                            all_count = resp.header_count;
                        }
                    }
                    h2_send_response_headers(
                        client_fd, &conn.encoder_dyn,
                        (uint32_t)completed_stream_id, resp.status,
                        all_hdrs, all_count,
                        0 /*no end_stream*/, conn.peer_max_frame_size
                    );
                    if (all_hdrs != resp.headers) free(all_hdrs);

                    int64_t stream_sw = s->send_window;
                    int64_t data_sent = h2_send_response_data(
                        client_fd, (uint32_t)completed_stream_id,
                        resp.body, resp.body_len, 1 /*end_stream*/,
                        conn.peer_max_frame_size,
                        &conn.conn_send_window, &stream_sw
                    );
                    s->send_window = stream_sw;
                    if (data_sent < 0) {
                        // Flow control exhausted — send RST_STREAM and continue
                        h2_send_rst_stream(client_fd, (uint32_t)completed_stream_id,
                                           H2_ERROR_FLOW_CONTROL_ERROR);
                    }
                }

                h2_response_fields_free(&resp);

                s->state = H2_STREAM_CLOSED;
                h2_conn_remove_closed_streams(&conn);
            }
        }
    }

h2_conn_done:
    if (!conn.goaway_sent) {
        h2_send_goaway(client_fd, conn.last_peer_stream_id, H2_ERROR_NO_ERROR, "");
    }
    h2_conn_free(&conn);
    #undef conn
    free(connp);
}

typedef struct { int64_t requests; } H2ServeResult;

// ── taida_net_h2_serve ─────────────────────────────────────────────────────
// Full HTTP/2 server loop: bind → accept → TLS handshake → ALPN check → serve.
// max_requests=0 means unlimited. Returns request count and connection count.
static H2ServeResult taida_net_h2_serve(int port, taida_val handler, int handler_arity,
                                         int64_t max_requests, int64_t timeout_ms,
                                         const char *cert_path, const char *key_path) {
    H2ServeResult fail_result = {-1};

    // Load OpenSSL (required for h2 — h2c is out of scope)
    if (!taida_ossl_load()) {
        return fail_result;
    }

    // Create TLS context with ALPN h2 / http/1.1
    char errbuf[512];
    OSSL_SSL_CTX *ssl_ctx = taida_tls_create_ctx_h2(cert_path, key_path, errbuf, sizeof(errbuf));
    if (!ssl_ctx) {
        return fail_result;
    }

    // Bind to 127.0.0.1:port
    int sockfd = socket(AF_INET, SOCK_STREAM, 0);
    if (sockfd < 0) { taida_ossl.SSL_CTX_free(ssl_ctx); return fail_result; }
    int opt = 1;
    setsockopt(sockfd, SOL_SOCKET, SO_REUSEADDR, &opt, sizeof(opt));
    struct sockaddr_in addr;
    memset(&addr, 0, sizeof(addr));
    addr.sin_family = AF_INET;
    inet_pton(AF_INET, "127.0.0.1", &addr.sin_addr);
    addr.sin_port = htons((unsigned short)port);
    if (bind(sockfd, (struct sockaddr*)&addr, sizeof(addr)) < 0) {
        close(sockfd); taida_ossl.SSL_CTX_free(ssl_ctx); return fail_result;
    }
    if (listen(sockfd, 128) < 0) {
        close(sockfd); taida_ossl.SSL_CTX_free(ssl_ctx); return fail_result;
    }

    int64_t request_count = 0;
    int64_t connection_count = 0;
    signal(SIGPIPE, SIG_IGN);

    while (max_requests == 0 || request_count < max_requests) {
        // Accept with timeout so we can re-check request count
        struct timeval tv;
        tv.tv_sec = 0;
        tv.tv_usec = 100000; // 100ms
        setsockopt(sockfd, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv));

        struct sockaddr_in peer_addr;
        socklen_t peer_len = sizeof(peer_addr);
        int client_fd = accept(sockfd, (struct sockaddr*)&peer_addr, &peer_len);
        if (client_fd < 0) {
            if (errno == EAGAIN || errno == EWOULDBLOCK || errno == EINTR) continue;
            break;
        }

        // TLS handshake
        {
            struct timeval to;
            to.tv_sec = (timeout_ms > 0) ? timeout_ms / 1000 : 30;
            to.tv_usec = (timeout_ms > 0) ? (timeout_ms % 1000) * 1000 : 0;
            setsockopt(client_fd, SOL_SOCKET, SO_RCVTIMEO, &to, sizeof(to));
            setsockopt(client_fd, SOL_SOCKET, SO_SNDTIMEO, &to, sizeof(to));
        }

        OSSL_SSL *ssl = taida_tls_handshake(ssl_ctx, client_fd);
        if (!ssl) { close(client_fd); continue; }

        // ALPN check: only proceed if "h2" was negotiated
        int h2_negotiated = 0;
        if (taida_ossl.SSL_get0_alpn_selected) {
            const unsigned char *alpn_data = NULL;
            unsigned int alpn_len = 0;
            taida_ossl.SSL_get0_alpn_selected(ssl, &alpn_data, &alpn_len);
            if (alpn_data && alpn_len == 2 &&
                alpn_data[0] == 'h' && alpn_data[1] == '2') {
                h2_negotiated = 1;
            }
        } else {
            // ALPN API not available — assume h2 (only h2 clients should connect here)
            h2_negotiated = 1;
        }

        if (!h2_negotiated) {
            // No silent fallback: close connection per design policy
            taida_tls_shutdown_free(ssl);
            close(client_fd);
            continue;
        }

        connection_count++;
        // NB6-47: emit connection count to stderr (side channel for benchmarks).
        // This keeps the public result pack contract clean (@(requests: Int) only).
        fprintf(stderr, "[h2-conn] %lld\n", (long long)connection_count);

        // Set TLS for this connection's I/O
        tl_ssl = ssl;

        // Get peer info
        char peer_host[64];
        int peer_port_val = ntohs(peer_addr.sin_port);
        if (!inet_ntop(AF_INET, &peer_addr.sin_addr, peer_host, sizeof(peer_host))) {
            snprintf(peer_host, sizeof(peer_host), "127.0.0.1");
        }

        H2ServeCtx serve_ctx;
        serve_ctx.handler = handler;
        serve_ctx.handler_arity = handler_arity;
        serve_ctx.request_count = &request_count;
        serve_ctx.max_requests = max_requests;
        snprintf(serve_ctx.peer_host, sizeof(serve_ctx.peer_host), "%s", peer_host);
        serve_ctx.peer_port = peer_port_val;

        taida_net_h2_serve_connection(client_fd, &serve_ctx);

        // TLS shutdown — bidirectional: first call sends close-notify,
        // second call waits for peer's close-notify (or EAGAIN/EWOULDBLOCK).
        // This ensures all buffered response data reaches the client before
        // the TCP connection is torn down (avoids RST truncating the response).
        if (ssl) {
            int sd1 = taida_ossl.SSL_shutdown(ssl);
            if (sd1 == 0) {
                // First shutdown sent, wait for peer. Drain incoming bytes.
                unsigned char drain_buf[256];
                int drain_attempts = 0;
                while (drain_attempts++ < 20) {
                    int r = taida_ossl.SSL_read(ssl, drain_buf, (int)sizeof(drain_buf));
                    if (r <= 0) break;
                }
                taida_ossl.SSL_shutdown(ssl); // second call — receive peer's close-notify
            }
            taida_ossl.SSL_free(ssl);
        }
        tl_ssl = NULL;
        // TCP half-close + brief drain to ensure kernel flushes send buffer.
        shutdown(client_fd, SHUT_WR);
        {
            unsigned char tcp_drain[256];
            struct timeval tv2 = {0, 50000}; // 50ms
            setsockopt(client_fd, SOL_SOCKET, SO_RCVTIMEO, &tv2, sizeof(tv2));
            int d;
            while ((d = (int)recv(client_fd, tcp_drain, sizeof(tcp_drain), 0)) > 0) {}
        }
        close(client_fd);
    }

    close(sockfd);
    taida_ossl.SSL_CTX_free(ssl_ctx);
    H2ServeResult ok_result = {request_count};
    return ok_result;
}

