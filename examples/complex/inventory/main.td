// main.td -- Inventory management demo
//
// Multi-module system: models -> operations -> display -> main
// Demonstrates: type inheritance (custom + error), throw/|==,
//   molds, pipelines, Lax, conditional branches, modules

>>> ./models.td => @(mkStocked, mkPerishable)
>>> ./operations.td => @(findByName, calcTotalValue, lowStockReport, expensiveReport, expiringReport, validateStock)
>>> ./display.td => @(printItems, printPerishable)

stdout("=== Inventory System ===")
stdout("")

// Build inventory — mix of StockedItem and PerishableItem
items <= @[
  mkStocked("Widget A", 50, 10),
  mkStocked("Widget B", 150, 3),
  mkStocked("Widget C", 200, 0),
  mkStocked("Widget D", 30, 20)
]

perishables <= @[
  mkPerishable("Milk", 5, 12, 2),
  mkPerishable("Bread", 3, 8, 5),
  mkPerishable("Eggs", 4, 0, 1)
]

printItems("--- All Items ---", items)

// Perishable items display
stdout("--- Perishable Items ---")
Map[perishables, printPerishable]() ]=> shown
stdout("")

// Total value
total <= calcTotalValue(items)
stdout("Total inventory value: $" + total.toString())
stdout("")

// Low stock alert
lowItems <= lowStockReport(items)
printItems("--- Low Stock (qty < 5) ---", lowItems)

// Expensive items
expItems <= expensiveReport(items)
printItems("--- Expensive (price > $100) ---", expItems)

// Expiring items
expiring <= expiringReport(perishables)
stdout("--- Expiring Soon ---")
Map[expiring, printPerishable]() ]=> exShown
stdout("")

// Find item by name
found <= findByName(items, "Widget B")
| found.hasValue |>
  stdout("Found: " + found.__value.name + " ($" + found.__value.price.toString() + ", qty: " + found.__value.qty.toString() + ")")
| _ |>
  stdout("Item not found")

stdout("")

// Validate stock — throw/catch handled inside validateStock
stdout("--- Stock Validation ---")
stdout(validateStock(items.get(0).__value))
stdout(validateStock(items.get(1).__value))
stdout(validateStock(items.get(2).__value))

stdout("")
stdout("=== Done ===")
