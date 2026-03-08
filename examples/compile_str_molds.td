// compile_str_molds.td — Native string operation mold tests

// Upper / Lower
msg <= "Hello World"
upper_msg <= Upper[msg]()
stdout(upper_msg)

lower_msg <= Lower[msg]()
stdout(lower_msg)

// Trim
padded <= "  hello  "
trimmed <= Trim[padded]()
stdout(trimmed)

// Replace (first only — default)
text <= "foo bar foo"
replaced <= Replace[text, "foo", "baz"]()
stdout(replaced)

// Replace (all)
replaced_all <= Replace[text, "foo", "baz"](all <= true)
stdout(replaced_all)

// CharAt
greeting <= "Hello"
ch <= CharAt[greeting, 1]()
stdout(ch)

// Repeat
star <= Repeat["*", 5]()
stdout(star)

// Reverse
rev <= Reverse["abcde"]()
stdout(rev)

// Slice with start/end
sliced <= Slice["Hello World"](start <= 0, end <= 5)
stdout(sliced)

// Pad (default: start padding with spaces)
padded_num <= Pad["42", 5]()
stdout(padded_num)
