# SciFact baseline fixture

SciFact, BEIR test retrieval split, downloaded 2026-10-14. The release is
identified by the BEIR archive MD5 `5f7d1de60b170fc8027bb7898e2efca1` and
SHA256 `536e14446a0ba56ed1398ab1055f39fe852686ecad24a6306c80c490fa8e0165`.

The entire 5,183-document corpus is retained. There are 300 judged test
queries and 339 test qrels. The data files total 8,031,708 bytes, below
10 MB. No corpus subsampling occurred. Only queries named by the test qrels
are retained, in upstream order. The loader rejects missing judgment
references, duplicate IDs, malformed rows and negative relevance values.

[Upstream SciFact](https://github.com/allenai/scifact) licenses claims and
annotations under CC BY 4.0 and the S2ORC abstracts under ODC-By 1.0.
`LICENSE` contains attribution, change notices and both license texts.
NFCorpus is smaller but its homepage allows academic purposes only and
requires permission for other uses. SciFact is smaller than ArguAna and is
the smallest qualifying candidate in the supplied BEIR table.

Download:

```sh
curl --fail --location https://public.ukp.informatik.tu-darmstadt.de/thakur/BEIR/datasets/scifact.zip -o scifact.zip
sha256sum scifact.zip
```

Reproduce the conversion with Python 3 from this directory:

```python
import json
from pathlib import Path
from zipfile import ZipFile

with ZipFile('scifact.zip') as archive:
    rows = archive.read('scifact/qrels/test.tsv').decode().splitlines()[1:]
    judged_queries = {row.split('\t')[0] for row in rows}
    for name, keys in [('corpus', ['id', 'title', 'text']),
                       ('queries', ['id', 'text'])]:
        output = []
        for line in archive.read(f'scifact/{name}.jsonl').splitlines():
            record = json.loads(line)
            record['id'] = record.pop('_id')
            if name == 'queries' and record['id'] not in judged_queries:
                continue
            output.append(json.dumps({k: record[k] for k in keys},
                                     ensure_ascii=True, separators=(',', ':')) + '\n')
        Path(f'{name}.jsonl').write_text(''.join(output))
    Path('qrels.tsv').write_text('query_id\tdocument_id\trelevance\n' +
                                '\n'.join(rows) + '\n')
```

SHA256 of committed data:

- `corpus.jsonl`: `6c0b91ddc28c70283fc5ad859cf2d66642684226a3915e0c1ff588df1cc2b4a5`
- `queries.jsonl`: `004fa0c1ecb9a6876cd0563710b6e03fa0a2cc52e0185396bece3a0bdce4716a`
- `qrels.tsv`: `7ebd401e662e6154120cac9d06d6a4ca52b11a9330528adeef8655835c813f2b`

## Measurement contract

The runner uses engine-native ranking through `SearchEngine`, the builder's
query expansion, fusion depth and weighted reciprocal rank fusion, and
native Qdrant hybrid fusion. Both fields have weight 1. The baseline hybrid
ratio is explicitly 0.5. The core application default remains lexical
(ratio 0). All queries run once to warm each mode before the measured pass;
query parsing, expansion, retrieval, fusion and hybrid query inference are
timed. One vocabulary snapshot is loaded after indexing and before warm-up
and reused because this generation is immutable. Records expose its
`vocabulary_prepare_ms` separately and bind the budgets to `latency_scope`.
Corpus inference, indexing, vocabulary preparation and model hydration are
outside the warm engine latency distribution. These are direct engine
measurements; the generic builder also performs vocabulary lookup and model
hydration on its own path.
The isolated in-memory metadata pools disable idle and lifetime reaping so a long run
keeps its in-memory SQLite schema. There is no query embedding cache. Percentiles use nearest rank. nDCG@10
uses trec_eval's linear relevance gain and log2(rank + 1) discount, including
all fixture queries. Duplicate retrieved documents gain only once.

`baseline.json` starts **uncalibrated**. Missing rival scores and versions
are null, never measurements. The normal gate refuses it. Set
`SEARCH_BASELINE_ALLOW_UNCALIBRATED=1` only for provisional measurements;
this still enforces latency budgets. Rival superiority remains unverified
until calibration. The floor is the larger rival lexical score minus the
committed tolerance (0.05); hybrid must strictly exceed the larger score.
The fixture digest binds calibration, records and gates to the same bytes.

Corpus inference on this machine uses `SEARCH_EMBED_BATCH=8` to bound
padding and memory. It is outside the query latency window. Query inference
still receives one query per request, with the same fp32 model and profile.
Local budgets use unoptimized dev/test builds and twice the largest observed
p95, rounded up to whole seconds: collection 12,000 ms, SQLite 2,000 ms and
Qdrant 2,000 ms. `budget_comment` records the observations. Postgres/MySQL
limits are unmeasured provisional estimates until their test URLs are available.
The local default suite took 35 minutes; the three real-model engine tests
took 133 minutes, including corpus inference and both query modes.

## Calibrate on the developer's server

From the repository root, with all four `SEARCH_BASELINE_MEILISEARCH_URL`,
`SEARCH_BASELINE_MEILISEARCH_KEY`, `SEARCH_BASELINE_TYPESENSE_URL` and
`SEARCH_BASELINE_TYPESENSE_KEY` variables exported from the server's secure
configuration:

```sh
CARGO_INCREMENTAL=0 /home/shawn/workspace2/scratchpads/cargo-slots/cargo-slot cargo run -p app --bin console -- search:baseline calibrate
```

No keys belong in command arguments or committed files. Calibration uses
unique temporary indexes/collections, waits for all Meilisearch tasks,
checks every Typesense import result and cleans up its own resources.
It writes the file only when both services succeed. It preserves the
committed local latency budgets; `SEARCH_BASELINE_TOLERANCE` can explicitly
set the tolerance for the replacement file. Review and commit the generated
`baseline.json`; the command does not stage or commit changes.

Run and gate:

```sh
SEARCH_BASELINE_ALLOW_UNCALIBRATED=1 CARGO_INCREMENTAL=0 /home/shawn/workspace2/scratchpads/cargo-slots/cargo-slot cargo run -p app --bin console -- search:baseline run --engine collection
SEARCH_BASELINE_ALLOW_UNCALIBRATED=1 CARGO_INCREMENTAL=0 /home/shawn/workspace2/scratchpads/cargo-slots/cargo-slot cargo nextest run -p suprnova --test search -E 'test(/baseline/)'
SEARCH_EMBEDDER=fastembed SEARCH_EMBED_BATCH=8 SEARCH_MODEL_CACHE=/home/shawn/workspace2/scratchpads/search-models SEARCH_BASELINE_ALLOW_UNCALIBRATED=1 CARGO_INCREMENTAL=0 /home/shawn/workspace2/scratchpads/cargo-slots/cargo-slot cargo nextest run -p suprnova --features search-fastembed --test search --run-ignored only -E 'test(/baseline_.*hybrid/)'
```

Remove the uncalibrated override after calibration. Records are written to
`target/search-baseline/` before applying floors so failures retain evidence.
Every engine writes `<engine>.json`; hybrid tests also preserve
`<engine>-hybrid.json`.
Postgres/MySQL tests require disposable `PG_TEST_URL` / `MYSQL_TEST_URL`.
The Qdrant test uses `QDRANT_TEST_URL`, with SQLite for metadata.
The console accepts collection, sqlite, postgres, mysql and qdrant; no engine
argument runs collection and sqlite. SQLite and Qdrant console measurements
use a separate in-memory metadata connection. Postgres/MySQL console
measurements require the corresponding disposable test URL.
The Qdrant console uses `QDRANT_URL` and optional `QDRANT_API_KEY` from the
normal engine configuration; its test uses `QDRANT_TEST_URL` instead.
# Tiny ONNX fixtures

`text.onnx` applies dynamic quantization to token IDs before producing two token features. Mixing low and high IDs changes the quantization range, so the test catches batch-dependent embeddings rather than checking a batch-size field.

`sparse.onnx` returns the three tensors fastembed's joint BGE-M3 loader expects. Its scalar initializer lives in `sparse.onnx_data`, so loading it exercises external tensor resolution. The tokenizer and metadata are deliberately small and support both fixtures.

Regenerate the graphs with `uv run --with onnx python framework/tests/search/fixtures/generate.py`. They use ONNX IR 8 and opset 13. These fixtures contain no pretrained weights and need no model download.
