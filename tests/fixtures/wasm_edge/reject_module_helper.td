// Helper module for reject_module_import test
greet <= fn name => :Str
  "hello " + name

<<< => @(greet)
