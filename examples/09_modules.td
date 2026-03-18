// Taida Lang Example: Module System
// Import from a local file

>>> ./module_utils.td => @(double, triple, greet)

// Use imported functions
result1 <= double(21)
stdout(result1)

result2 <= triple(14)
stdout(result2)

msg <= greet("Taida")
stdout(msg)
