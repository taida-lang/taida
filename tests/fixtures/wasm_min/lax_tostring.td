// W-5f: Lax/Div/Mod toString tests
// Lax with value
stdout(Lax[42]().toString())

// Lax with default (Div by zero)
stdout(Div[0, 0]().toString())

// Div with value
stdout(Div[10, 3]().toString())
