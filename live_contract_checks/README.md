# Live library contract observers

These Python checks observe the six requirements in
[the Live library contract](../docs/spec/live-library-contract.md).
They do not implement the package validator. The Rust contract crate must own
package rules and package identity.

Run the observer tests from the repository root:

```sh
python3 -B -m unittest discover -s live_contract_checks -t .
python3 -B -m live_contract_checks.run LCT-001
```

The second command currently exits 3: the real collector is not implemented.
Exit 0 means current framework observations satisfy the assertions. Exit 1
means an observed violation. Exit 3 means missing, malformed, stale, interrupted,
or incomplete evidence. Only complete real observations can print a pass line.

`subject.json` selects `framework` in normal use. `observer-control` takes a
requirement and synthetic observation from `control_examples.py`. A violating
control must fail the same assertions used for real observations. A satisfying
control still exits 3. Controls test the observer; they never qualify Suprnova.

`cases.py` lists minimum coverage. The collector must discover and run the entire
shared corpus, including cases added beyond this list. Golden diagnostic codes
and digest vectors must be authored independently of observed validator results.
`control.rejected` and `control-only` belong only to synthetic examples.

The runner bounds combined collector output to 1 MiB and execution to 600 seconds.
It captures both output pipes without forwarding child text into Sudus results.
It kills and reaps the collector process group on completion, timeout, excessive
output, or interruption. The implementation requires Python 3.10+ and POSIX
process groups. Tests cover subprocess cleanup as well as assertion failures.

The runner and checkout identity helper adapt the existing SDK observer pattern.
They are local to this framework repository and do not import SDK tooling. The
configured SDK checkout is evidence for the ownership review, not executable
observer code. [Collector requirements](../proof/README.md) define the remaining
implementation and evidence boundary.
