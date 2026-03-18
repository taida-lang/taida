// RC-1p: Self-import test
// Expected: Circular import detected error
>>> ./main_self.td => @(something)
stdout(something())
