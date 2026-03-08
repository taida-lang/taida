// === Taida Lang: No-argument Functions ===

// Basic no-arg function
getVersion =
  "1.0.0"
=> :Str

stdout(`Version: ${getVersion()}`)

// No-arg function with multiple statements
getConfig =
  host <= "localhost"
  port <= 8080
  `${host}:${port}`
=> :Str

stdout(`Config: ${getConfig()}`)

// No-arg returning a buchi pack
getDefaults =
  @(name <= "default", value <= 0, active <= false)
=> :@(name: Str, value: Int, active: Bool)

defaults <= getDefaults()
stdout(`Defaults: name=${defaults.name}, value=${defaults.value}`)

// No-arg with condition
getEnvironment =
  mode <= "production"
  | mode == "production" |> "PROD"
  | mode == "development" |> "DEV"
  | _ |> "UNKNOWN"
=> :Str

stdout(`Environment: ${getEnvironment()}`)
