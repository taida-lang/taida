// operations.td -- Inventory operations (search, filter, aggregation)
//
// Demonstrates: molds (Filter, Map, Find, Sum), Lax, throw/|==

>>> ./models.td => @(StockedItem, StockError, totalValue, isLowStock, isExpensive, isExpiring)

findByName items name =
  Find[items, _ item = item.name == name]()
=> :Lax

calcTotalValue items =
  Map[items, totalValue]() ]=> values
  Sum[values]()
=> :Int

lowStockReport items =
  Filter[items, isLowStock]() ]=> result
  result

expensiveReport items =
  Filter[items, isExpensive]() ]=> result
  result

expiringReport items =
  Filter[items, isExpiring]() ]=> result
  result

// Validate stock level — throws StockError if qty is 0, catches internally
validateStock item =
  |== err: Error =
    "Error: " + err.message + " [" + err.reason + ": " + err.item + "]"
  => :Str
  | item.qty == 0 |>
    err <= StockError(message <= "Out of stock", reason <= "zero quantity", item <= item.name)
    err.throw()
  | _ |>
    item.name + " OK"
=> :Str

<<< @(findByName, calcTotalValue, lowStockReport, expensiveReport, expiringReport, validateStock)
