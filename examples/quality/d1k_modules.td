// D-1k: docs/guide/10_modules.md code examples

// Import from relative path
>>> ./d1k_modules_lib.td => @(helper, FORMAT_PREFIX)

result <= helper(5)
stdout(FORMAT_PREFIX + result.toString())
