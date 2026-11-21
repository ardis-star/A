# V1.1.0 bytecode compatibility fixtures

The five hex files are exact `.avm` images built from the respective
`examples/*.pa` files using the unmodified V1.1.0 source at base commit
`b7e617c21ab06eaa0b82de583b6761518780602a`. They were generated using
`plus-a build examples/<name>.pa -o <name>.avm`, then hex encoded. The V1.1.0
interpreter produced statuses 125, 42, 0, 0, and 100 respectively. The
`v11_example_bytecode_remains_compatible` test verifies and executes these
images against the current implementation. This is representative compatibility
coverage, not a proof for all historical or malformed bytecode.
