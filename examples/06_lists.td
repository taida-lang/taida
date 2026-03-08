// === Taida Lang: Lists & Methods ===

numbers <= @[10, 3, 7, 1, 9, 5, 2, 8, 4, 6]

stdout(`List: ${numbers}`)
stdout(`Length: ${numbers.length()}`)
numbers.first() ]=> first_val
numbers.last() ]=> last_val
stdout(`First: ${first_val}`)
stdout(`Last: ${last_val}`)
sum_val <= Sum[numbers]()
stdout(`Sum: ${sum_val}`)
sorted_val <= Sort[numbers]()
stdout(`Sorted: ${sorted_val}`)
reversed_val <= Reverse[numbers]()
stdout(`Reversed: ${reversed_val}`)
stdout(`Contains 7: ${numbers.contains(7)}`)
stdout(`Contains 42: ${numbers.contains(42)}`)
unique_val <= Unique[@[1, 2, 2, 3, 3, 3]]()
stdout(`Unique: ${unique_val}`)

// String methods
text <= "  Hello, Taida Lang!  "
trimmed <= Trim[text]()
upper_val <= Upper[trimmed]()
lower_val <= Lower[trimmed]()
replaced_val <= Replace[trimmed, "Taida", "World"]()
stdout(`Original: '${text}'`)
stdout(`Trimmed: '${trimmed}'`)
stdout(`Upper: '${upper_val}'`)
stdout(`Lower: '${lower_val}'`)
stdout(`Length: ${trimmed.length()}`)
stdout(`Contains 'Taida': ${text.contains("Taida")}`)
stdout(`Replace: '${replaced_val}'`)
