// compile_num_molds.td — Native numeric operation mold tests

// ToFixed — returns Str directly
pi <= 3.14159
fixed <= ToFixed[pi, 2]()
stdout(fixed)

fixed3 <= ToFixed[2.71828, 3]()
stdout(fixed3)

// Abs (Int)
neg <= -42
abs_int <= Abs[neg]()
stdout(abs_int.toString())

// Floor
floored <= Floor[3.7]()
stdout(floored.toString())

floored_neg <= Floor[-2.3]()
stdout(floored_neg.toString())

// Ceil
ceiled <= Ceil[3.2]()
stdout(ceiled.toString())

ceiled_neg <= Ceil[-2.7]()
stdout(ceiled_neg.toString())

// Round
rounded <= Round[3.5]()
stdout(rounded.toString())

rounded_down <= Round[3.4]()
stdout(rounded_down.toString())

// Truncate
truncated <= Truncate[3.9]()
stdout(truncated.toString())

truncated_neg <= Truncate[-3.9]()
stdout(truncated_neg.toString())

// Clamp (Int)
clamped <= Clamp[15, 0, 10]()
stdout(clamped.toString())

clamped_low <= Clamp[-5, 0, 10]()
stdout(clamped_low.toString())

clamped_mid <= Clamp[5, 0, 10]()
stdout(clamped_mid.toString())
