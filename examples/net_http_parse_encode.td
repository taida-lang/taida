// net_http_parse_encode.td -- HTTP parse and encode using taida-lang/net
//
// Demonstrates httpParseRequestHead and httpEncodeResponse without running a server.
// Works on all 3 backends: Interpreter, JS, Native.

>>> taida-lang/net => @(httpParseRequestHead, httpEncodeResponse)

// Parse a raw HTTP request
bytesLax <= Bytes["GET /hello?name=taida HTTP/1.1\r\nHost: localhost\r\nAccept: text/plain\r\n\r\n"]()
bytesLax ]=> bytes
parseResult <= httpParseRequestHead(bytes)
parseResult ]=> parsed

// Note: use consumed (Int) instead of complete (Bool) for 3-backend parity
stdout(parsed.consumed)
stdout(parsed.method.start)
stdout(parsed.method.len)
stdout(parsed.path.len)
stdout(parsed.query.len)

// Encode a response
resp <= @(
  status <= 200,
  headers <= @[@(name <= "content-type", value <= "text/plain")],
  body <= "Hello, taida!"
)
encodeResult <= httpEncodeResponse(resp)
encodeResult ]=> encoded
stdout(encoded.bytes.length())
