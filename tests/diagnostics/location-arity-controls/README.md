# Existing arity diagnostic controls

These exact PRINT and INPUT test sources were captured independently from
the original G1 2.4.2 firmware, twice each. The captures agree exactly:
PRINT stops at the first excess-argument comma (3:10); INPUT stops at the
comma introducing its seventh argument (4:26). They justify updating the
pre-existing compiler test positions without changing their rejection verdicts.

The fixtures are checked alongside `../location-probes` by the offline
`ppl-compat` diagnostic parity regression test. References were generated
by `ppl diag-parity capture`; do not edit them to accommodate native behavior.
