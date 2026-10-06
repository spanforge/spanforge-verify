These expanded JSON and JUnit fixtures freeze the specification's wire shape.
They are synthetic contracts, not execution evidence. Metadata and hashes are
intentional placeholders. `tools/generate_contract_fixtures.py` regenerates them.
The generator is a development tool; the Rust CLI has no Python runtime dependency.
