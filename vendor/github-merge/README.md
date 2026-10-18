# github-merge.py

Bitcoin Core's merge script, as dv runs it (`Space g` → merge in a PR, `M M` in `dv list`),
unchanged from upstream:

- source: https://github.com/bitcoin-core/bitcoin-maintainer-tools/blob/1519ef4883f89d7eb0c03134b2ae2878eff87988/github-merge.py
- SHA-256: `37e585e5c9bf354028e43b04b3833b3cea2010c15fdb01e48c4aa32bc86736fe`
- license: MIT (`COPYING`, from the same commit)

dv builds it in and writes it to its data directory to run it, so nothing is downloaded when you
merge. A new version comes only with a dv release, as a diff to this file. To use your own copy
instead (say, your checkout of bitcoin-maintainer-tools): `git config dv.mergescript <path>`.
