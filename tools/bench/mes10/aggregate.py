# SPDX-License-Identifier: Apache-2.0
"""Aggregation of MES-10 (quality of the fusion) and MES-11 (French analysis).

It reads the raw documents of the harness
(`crates/crible/examples/hybrid_eval.rs`, `data/mes10/raw/`), recomputes every
figure — recall@10, MRR@10, median rank, the paired bootstrap against `rrf` —
checks the coherence controls of `alpha`, and writes the two dated artefacts
next to their reports. It prints the Markdown tables of both reports.

Protocol: ADR-0092.

- decision 1: the main game is the 57 labelled questions, averaged over the 56
  satisfiable ones; `Q30` is reported apart and never enters a mean;
- decision 2: ten **documents** (`group_by: "document"`, `k = 10`); recall@10 is
  the mean of |expected ∩ top-10| / |expected|, MRR@10 the mean of 1/rank of the
  first expected document, zero when none is in the top 10; the secondary line
  gives the median rank of the questions that were found;
- decision 3: the binary column is the production default, the exact column is
  the ceiling; `alpha = 0` must reproduce the lexical branch alone and
  `alpha = 1` the dense one;
- decision 6: the decision metric is recall@10, tie-broken by MRR@10 then by the
  `alpha` closest to 0.5; the test is a paired bootstrap by question, 10 000
  resamples, fixed seed, 95 % interval of the difference against `rrf`; another
  configuration is only recommended when the lower bound is above zero, without
  regression on the secondary game, nor beyond the noise on
  `juri_texte_integral` and `textes_vigueur`; a tie is a valid conclusion and
  leaves `rrf` as the default;
- decision 5: MES-11 measures a replica of the analysis chain, one tantivy index
  per variant, anchored to the engine by the V0 document top-10.

Run from the repository root::

    (cd tools && uv run --no-sync python bench/mes10/aggregate.py \
        --commit "$(git rev-parse HEAD)" --date 2026-10-28 \
        --machine "MacBook Air M3, 8 cores, 24 GB, macOS 15.7.7, native" \
        --write)
"""

from __future__ import annotations

import argparse
import hashlib
import json
import random
import statistics
from pathlib import Path
from typing import Iterable, Mapping, Sequence

#: Root of the repository: a path recorded by the harness is made relative to
#: it, so a versioned artefact never carries a local absolute path.
ROOT = Path(__file__).resolve().parents[3]
#: Raw documents of the campaign.
DEFAULT_RAW = ROOT / "data" / "mes10" / "raw"
#: Where the dated artefacts go.
DEFAULT_MEASURES = ROOT / "docs" / "mesures"
#: File names of the two reports and of their artefacts (ADR-0092, decision 7).
SLUG_10 = "MES-10-qualite-fusion"
SLUG_11 = "MES-11-analyse-francaise"
#: The six structures of the sample, named as their index is.
STRUCTURES = ("articles", "decisions", "jorf", "sarde", "dole", "titres")
#: Methods of the campaign, in the order of the report. The names are the ones
#: the harness writes in `methods`: `relative_0` for `alpha = 0`, `relative_1`
#: for `alpha = 1`.
METHODS = (
    "dense",
    "lexical",
    "rrf",
    "relative_0",
    "relative_0.3",
    "relative_0.5",
    "relative_0.75",
    "relative_1",
)
#: Variants of MES-11.
VARIANTS = ("V0", "V1", "V2", "V3")
#: Questions the report excludes from every mean (ADR-0092, decision 1).
UNSATISFIABLE = "Q30"
#: Resamples of the paired bootstrap and its seed (ADR-0092, decision 6).
RESAMPLES = 10_000
SEED = 20261028
#: One question of 56 is worth this much, the resolution the report declares.
RESOLUTION = 1.0 / 56.0


# ------------------------------------------------------------------ metrics -----


def question_scores(
    expected: Sequence[str], top: Sequence[str]
) -> tuple[float, float, int | None]:
    """Recall, reciprocal rank and rank of one question, at the document level.

    `expected` is the list of expected external identifiers, `top` the ordered
    list of returned documents. The recall is the share of expected documents the
    top-10 holds; the rank is the position of the **first** expected document, from
    1, or `None` when none is in the top 10 (ADR-0092, decision 2).
    """
    if not expected:
        return 0.0, 0.0, None
    found = {document for document in top if document in set(expected)}
    recall = len(found) / len(expected)
    rank = next(
        (position + 1 for position, document in enumerate(top) if document in set(expected)),
        None,
    )
    return recall, (0.0 if rank is None else 1.0 / rank), rank


def summarise(rows: Iterable[Mapping[str, object]], method: str) -> dict[str, object]:
    """Recall@10, MRR@10 and median rank of the questions that were found.

    Only `rows` with a non-empty `expected` enter the mean; the caller decides
    which questions those are, so that `Q30` never slips into a mean.
    """
    recalls: list[float] = []
    reciprocal: list[float] = []
    ranks: list[int] = []
    for row in rows:
        expected = row["expected"]
        top = row["top"][method]  # type: ignore[index]
        recall, inverse, rank = question_scores(expected, top)  # type: ignore[arg-type]
        recalls.append(recall)
        reciprocal.append(inverse)
        if rank is not None:
            ranks.append(rank)
    return {
        "n": len(recalls),
        "recall": statistics.fmean(recalls) if recalls else 0.0,
        "mrr": statistics.fmean(reciprocal) if reciprocal else 0.0,
        "found": len(ranks),
        "median_rank": statistics.median(ranks) if ranks else None,
    }


def per_question(rows: Iterable[Mapping[str, object]], method: str) -> list[float]:
    """Recall of every question, in order, for the paired bootstrap."""
    values: list[float] = []
    for row in rows:
        recall, _, _ = question_scores(row["expected"], row["top"][method])  # type: ignore[arg-type, index]
        values.append(recall)
    return values


def bootstrap(
    other: Sequence[float],
    reference: Sequence[float],
    resamples: int = RESAMPLES,
    seed: int = SEED,
) -> dict[str, float]:
    """Paired bootstrap of the mean difference `other - reference`.

    The questions are resampled with replacement, the difference of the means is
    recomputed on every resample, and the interval is the 2.5 % and 97.5 %
    percentile of the 10 000 differences. A fixed seed makes the interval
    reproducible. The sign convention is the one of ADR-0092, decision 6: a
    positive difference means the other method is **better** than the reference,
    so a recommendation needs a lower bound above zero.
    """
    if len(reference) != len(other) or not reference:
        raise ValueError("the two series must have the same non-zero length")
    differences = [left - right for left, right in zip(other, reference)]
    generator = random.Random(seed)
    count = len(differences)
    means: list[float] = []
    for _ in range(resamples):
        total = 0.0
        for _ in range(count):
            total += differences[generator.randrange(count)]
        means.append(total / count)
    means.sort()
    # Percentile of the empirical distribution, nearest rank: the 2.5 % and
    # 97.5 % order statistics of the 10 000 resampled differences.
    low = means[max(0, int(0.025 * resamples) - 1)]
    high = means[min(resamples - 1, int(0.975 * resamples) - 1)]
    return {
        "difference": statistics.fmean(differences),
        "low": low,
        "high": high,
        "resamples": resamples,
        "seed": seed,
    }


# ------------------------------------------------------------------- inputs -----


def load_questions(path: Path) -> dict[str, dict]:
    """Questions of `queries.jsonl` by identifier, with their expected documents."""
    questions: dict[str, dict] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        row = json.loads(line)
        questions[row["id"]] = {
            "id": row["id"],
            "corpus": row.get("corpus"),
            "structure": row.get("structure"),
            "satisfiable": bool(row.get("satisfiable")),
            "labelled": str(row["id"]).startswith("Q"),
            "expected_in_sample": bool(row.get("expected_in_sample")),
            "expected": [
                entry["src_id"] for entry in row.get("expected") or [] if entry.get("src_id")
            ],
        }
    return questions


def load_campaign(raw: Path, column: str) -> list[dict]:
    """Every question measured in the column, with its index and its tops."""
    rows: list[dict] = []
    for structure in STRUCTURES:
        path = raw / f"eval-{structure}-{column}.json"
        if not path.is_file():
            raise SystemExit(f"missing raw document {path.name}")
        document = json.loads(path.read_text(encoding="utf-8"))
        for row in document["questions"]:
            row["index"] = document["index"]
            row["column"] = document["column"]
            row["pool_size"] = document["pool_size"]
            row["k"] = document["k"]
            row["grouping"] = document["grouping"]
            row["passages_per_document"] = document["passages_per_document"]
            row["candidates_per_branch"] = document["candidates_per_branch"]
            rows.append(row)
    return rows


def sha256(path: Path) -> str:
    """SHA-256 of a file, hex, as ADR-0092, decision 3, asks for the vectors."""
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


# -------------------------------------------------------------- decision rule --


def decide(
    methods: Mapping[str, Mapping[str, object]],
    bootstrap: Mapping[str, Mapping],
    reference: str = "rrf",
    secondary: Mapping[str, Mapping] | None = None,
    guardrails: Mapping[str, Mapping[str, Mapping]] | None = None,
) -> dict:
    """Applies the decision rule of ADR-0092, decision 6, and names the winner.

    The order is recall@10, then MRR@10, then the `alpha` closest to 0.5. A method
    other than the reference is only recommended when its lower bound against the
    reference is above zero; otherwise the reference stays the default and the
    report says why. When the secondary set and the guardrail corpora are given,
    a method that regresses there is rejected too: the secondary interval must not
    exclude zero on the negative side, and neither must a guardrail interval.
    MES-10 compares against `rrf`; MES-11 compares the analysis variants against
    `V0`.
    """
    baseline = methods[reference]
    eligible: list[tuple[float, float, float, str]] = []
    rejected: list[dict] = []
    for name, result in methods.items():
        if name == reference:
            continue
        interval = bootstrap[name]
        reason = None
        if interval["low"] <= 0.0:
            reason = "lower bound not above zero against the reference"
        elif secondary is not None and not no_detectable_regression(secondary[name]):
            reason = "regression on the secondary set"
        elif guardrails is not None and any(
            not no_detectable_regression(corpus_interval)
            for corpus_interval in guardrails[name].values()
        ):
            reason = "regression beyond noise on a guardrail corpus"
        if reason is not None:
            rejected.append(
                {
                    "method": name,
                    "low": interval["low"],
                    "high": interval["high"],
                    "reason": reason,
                }
            )
            continue
        distance = abs(alpha_of(name) - 0.5)
        eligible.append((result["recall"], result["mrr"], -distance, name))
    eligible.sort(reverse=True)
    if not eligible:
        return {
            "recommended": reference,
            "reason": f"no method beats {reference} by a lower bound above zero",
            "rejected": rejected,
            "reference": baseline,
        }
    best = eligible[0]
    return {
        "recommended": best[3],
        "reason": (
            f"recall@10 above {reference} with a lower bound above zero, "
            "tie-break MRR then alpha"
        ),
        "rejected": rejected,
        "reference": baseline,
    }


def alpha_of(method: str) -> float:
    """`alpha` of a `relative_score` point, or 0 when it carries none."""
    if not method.startswith("relative_"):
        return 0.0
    return float(method.removeprefix("relative_"))


def guardrails(rows: Sequence[Mapping[str, object]], method: str) -> dict[str, object]:
    """Difference against `rrf` on the two corpora the rule protects."""
    out: dict[str, object] = {}
    for corpus in ("juri_texte_integral", "textes_vigueur"):
        subset = [row for row in rows if row["corpus"] == corpus]
        reference = per_question(subset, "rrf")
        other = per_question(subset, method)
        interval = bootstrap(other, reference)
        out[corpus] = {
            "n": len(subset),
            "rrf": statistics.fmean(reference),
            "method": statistics.fmean(other),
            **interval,
        }
    return out


def no_detectable_regression(interval: Mapping[str, float]) -> bool:
    """Whether an interval rules out a detectable regression of the method.

    The difference is `method - reference`, so a regression of the method is a
    negative difference. There is no detectable regression when the upper bound
    stays at or above zero: the interval does not exclude that the method is as
    good as the reference, even when it is worse.
    """
    return interval["high"] >= 0.0


# ------------------------------------------------------------------ tables -----


def markdown_table(
    rows: Iterable[Mapping[str, object]], columns: Sequence[tuple[str, str]]
) -> str:
    """A Markdown table from a header of `(key, title)` pairs."""
    lines = ["| " + " | ".join(title for _, title in columns) + " |"]
    lines.append("|" + "|".join("---" for _ in columns) + "|")
    for row in rows:
        cells = []
        for key, _ in columns:
            value = row[key]
            if isinstance(value, float):
                cells.append(f"{value:.4f}")
            elif value is None:
                cells.append("—")
            else:
                cells.append(str(value))
        lines.append("| " + " | ".join(cells) + " |")
    return "\n".join(lines)


# ----------------------------------------------------------------- campaign -----


def build_10(
    raw: Path,
    questions: Mapping[str, dict],
    commit: str,
    date: str,
    machine: str,
    vectors: Path,
    measures: Path,
) -> tuple[dict, list[str]]:
    """Recomputes MES-10 and returns its artefact and the report tables."""
    binary = load_campaign(raw, "binary")
    exact = load_campaign(raw, "exact")
    exact_by_id = {row["id"]: row for row in exact}

    labelled = [
        row
        | {
            "satisfiable": questions[row["id"]]["satisfiable"],
            "corpus": questions[row["id"]]["corpus"],
        }
        for row in binary
        if questions[row["id"]]["labelled"]
    ]
    measured = [row for row in labelled if row["id"] != UNSATISFIABLE]
    unsatisfiable = [row for row in labelled if row["id"] == UNSATISFIABLE]
    secondary = [
        row
        | {
            "corpus": questions[row["id"]]["corpus"],
            "satisfiable": questions[row["id"]]["satisfiable"],
        }
        for row in binary
        # Decision 1 counts the 145 questions at `expected_in_sample: true`,
        # labelled or not: `Q28` and `Q29` are labelled and belong to both sets.
        if questions[row["id"]]["expected_in_sample"]
    ]

    # The exact column is the dense branch alone: it replaces the `dense` top of
    # the binary column, so that a gap can be attributed to the fusion.
    for row in labelled + secondary:
        exact_row = exact_by_id.get(row["id"])
        if exact_row is None:
            raise SystemExit(f"the exact column has no row for {row['id']}")
        row["top"] = dict(row["top"])
        row["top"]["dense_exact"] = exact_row["top"]["dense"]

    summary: dict[str, dict] = {}
    for method in (*METHODS, "dense_exact"):
        summary[method] = summarise(measured, method)
    intervals = {
        method: bootstrap(per_question(measured, method), per_question(measured, "rrf"))
        for method in (*METHODS, "dense_exact")
        if method != "rrf"
    }
    exact_interval = intervals["dense_exact"]

    by_corpus: dict[str, dict[str, dict]] = {}
    for corpus in sorted({row["corpus"] for row in measured}):
        subset = [row for row in measured if row["corpus"] == corpus]
        by_corpus[corpus] = {
            method: summarise(subset, method) for method in (*METHODS, "dense_exact")
        }

    coherence = {
        "alpha_0_equals_lexical": sum(
            1 for row in measured if row["top"]["relative_0"] == row["top"]["lexical"]
        ),
        "alpha_1_equals_dense": sum(
            1 for row in measured if row["top"]["relative_1"] == row["top"]["dense"]
        ),
        "questions": len(measured),
    }

    secondary_summary = {
        method: summarise(secondary, method) for method in (*METHODS, "dense_exact")
    }
    secondary_intervals = {
        method: bootstrap(per_question(secondary, method), per_question(secondary, "rrf"))
        for method in (*METHODS, "dense_exact")
        if method != "rrf"
    }
    guardrail_results = {
        method: guardrails(measured, method)
        for method in (*METHODS, "dense_exact")
        if method != "rrf"
    }
    decision = decide(
        summary,
        intervals,
        secondary=secondary_intervals,
        guardrails=guardrail_results,
    )

    # The five variants of the « article 1240 » case are measured on the
    # `articles` index, which holds the expected document (ADR-0092, decision 4).
    article = json.loads((raw / "eval-articles-binary.json").read_text(encoding="utf-8")).get(
        "case_1240"
    )
    enrichment_path = ROOT / "data" / "mes10" / "extraction.json"
    enrichment = json.loads(enrichment_path.read_text(encoding="utf-8"))
    pool_sizes = {row["index"]: row["pool_size"] for row in binary}
    passages_measured = sum(pool_sizes.values())

    # Every raw document carries these four settings: read them instead of
    # repeating literals in the artefact, and refuse a campaign where they
    # disagree between structures.
    def single(field: str) -> object:
        values = {row[field] for row in binary + exact}
        if len(values) != 1:
            raise SystemExit(f"the raw documents disagree on '{field}': {values}")
        return values.pop()

    k = single("k")
    grouping = single("grouping")
    passages_per_document = single("passages_per_document")
    candidates_per_branch = single("candidates_per_branch")

    artefact = {
        "measure": "MES-10",
        "protocol": "ADR-0092",
        "date": date,
        "commit": commit,
        "machine": machine,
        "query_vectors": {
            "path": vectors.relative_to(ROOT).as_posix(),
            "sha256": sha256(vectors),
            "precision": "fp32",
            "model": "mdenseon",
        },
        "raw": {
            path.relative_to(ROOT).as_posix(): sha256(path)
            for path in sorted(raw.glob("eval-*.json"))
        },
        "k": k,
        "grouping": grouping,
        "passages_per_document": passages_per_document,
        "candidates_per_branch": candidates_per_branch,
        "enriched_set": {
            "path": enrichment_path.relative_to(ROOT).as_posix(),
            "sha256": sha256(enrichment_path),
            "documents": enrichment["totals"]["documents"],
            "passages": enrichment["totals"]["passages"],
            "structures": enrichment["structures"],
            "passages_measured": passages_measured,
            "pool_by_structure": pool_sizes,
        },
        "methods": list(METHODS) + ["dense_exact"],
        "main": {
            "questions": len(labelled),
            "averaged": len(measured),
            "excluded": [UNSATISFIABLE],
            "resolution": RESOLUTION,
            "summary": summary,
            "bootstrap": intervals,
            "by_corpus": by_corpus,
        },
        "controls": coherence,
        "secondary": {
            "questions": len(secondary),
            "summary": secondary_summary,
            "bootstrap": secondary_intervals,
        },
        "guardrails": guardrail_results,
        "decision": decision,
        "unsatisfiable": [
            {"id": row["id"], "expected": questions[row["id"]]["expected"], "top": row["top"]}
            for row in unsatisfiable
        ],
        "case_1240": article,
        "exact_column": {
            "note": "dense with exact_threshold at its maximum on a clone of the index",
            "interval_against_rrf": exact_interval,
        },
    }

    tables = render_10(artefact)
    if measures is not None:
        (measures / f"{SLUG_10}.{date}.json").write_text(
            json.dumps(artefact, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
        )
    return artefact, tables


def render_10(artefact: Mapping[str, object]) -> list[str]:
    """The Markdown tables of MES-10, in the order of the report."""
    main = artefact["main"]
    summary = main["summary"]
    intervals = main["bootstrap"]
    rows = []
    labels = {
        "dense": "dense BINARY",
        "dense_exact": "dense EXACT",
        "lexical": "lexical",
        "rrf": "rrf",
        "relative_0": "α = 0",
        "relative_0.3": "α = 0,3",
        "relative_0.5": "α = 0,5",
        "relative_0.75": "α = 0,75",
        "relative_1": "α = 1",
    }
    for method in ("dense", "dense_exact", "lexical", "rrf", *METHODS[3:]):
        result = summary[method]
        interval = intervals.get(method)
        rows.append(
            {
                "method": labels[method],
                "recall": result["recall"],
                "mrr": result["mrr"],
                "median": result["median_rank"],
                "found": result["found"],
                "difference": None if interval is None else interval["difference"],
                "ci": "—"
                if interval is None
                else f"[{interval['low']:+.4f} ; {interval['high']:+.4f}]",
            }
        )
    out = [
        "Campagne principale, 56 questions satisfiables, recall@10 au document :",
        "",
        markdown_table(
            rows,
            [
                ("method", "Mode"),
                ("recall", "recall@10"),
                ("mrr", "MRR@10"),
                ("median", "rang médian"),
                ("found", "trouvées"),
                ("difference", "écart à `rrf`"),
                ("ci", "IC 95 %"),
            ],
        ),
        "",
    ]
    corpus_rows = []
    for corpus, per_method in artefact["main"]["by_corpus"].items():
        for method in ("dense", "dense_exact", "lexical", "rrf", *METHODS[3:]):
            corpus_rows.append(
                {
                    "corpus": corpus,
                    "n": per_method[method]["n"],
                    "method": labels[method],
                    "recall": per_method[method]["recall"],
                    "mrr": per_method[method]["mrr"],
                }
            )
    out.extend(
        [
            "Par corpus, avec leur `n` :",
            "",
            markdown_table(
                corpus_rows,
                [
                    ("corpus", "Corpus"),
                    ("n", "n"),
                    ("method", "Mode"),
                    ("recall", "recall@10"),
                    ("mrr", "MRR@10"),
                ],
            ),
            "",
        ]
    )
    secondary = artefact["secondary"]
    secondary_rows = [
        {
            "method": labels[method],
            "recall": secondary["summary"][method]["recall"],
            "mrr": secondary["summary"][method]["mrr"],
            "median": secondary["summary"][method]["median_rank"],
            "found": secondary["summary"][method]["found"],
            "difference": None
            if method == "rrf"
            else secondary["bootstrap"][method]["difference"],
            "ci": "—"
            if method == "rrf"
            else f"[{secondary['bootstrap'][method]['low']:+.4f} ; "
            f"{secondary['bootstrap'][method]['high']:+.4f}]",
        }
        for method in ("dense", "dense_exact", "lexical", "rrf", *METHODS[3:])
    ]
    out.extend(
        [
            f"Jeu secondaire, {secondary['questions']} requêtes à `expected_in_sample: true` :",
            "",
            markdown_table(
                secondary_rows,
                [
                    ("method", "Mode"),
                    ("recall", "recall@10"),
                    ("mrr", "MRR@10"),
                    ("median", "rang médian"),
                    ("found", "trouvées"),
                    ("difference", "écart à `rrf`"),
                    ("ci", "IC 95 %"),
                ],
            ),
            "",
        ]
    )
    case = artefact.get("case_1240") or {}
    case_rows = []
    for variant in case.get("variants") or []:
        ranks = variant["rank"]
        case_rows.append(
            {
                "name": variant["name"],
                "text": variant["text"],
                "dense": ranks["dense"],
                "lexical": ranks["lexical"],
                "rrf": ranks["rrf"],
                "a0": ranks["relative_0"],
                "a03": ranks["relative_0.3"],
                "a05": ranks["relative_0.5"],
                "a075": ranks["relative_0.75"],
                "a1": ranks["relative_1"],
            }
        )
    out.extend(
        [
            "Cas « article 1240 », rang du document "
            f"`{case.get('expected', '—')}` sur l'index `articles` :",
            "",
            markdown_table(
                case_rows,
                [
                    ("name", "Variante"),
                    ("text", "Texte"),
                    ("dense", "dense"),
                    ("lexical", "lexical"),
                    ("rrf", "rrf"),
                    ("a0", "α = 0"),
                    ("a03", "α = 0,3"),
                    ("a05", "α = 0,5"),
                    ("a075", "α = 0,75"),
                    ("a1", "α = 1"),
                ],
            ),
            "",
        ]
    )
    unsatisfiable_rows = []
    for row in artefact["unsatisfiable"]:
        expected = row["expected"][0] if row["expected"] else None
        unsatisfiable_rows.append(
            {
                "id": row["id"],
                "expected": expected,
                "dense": None
                if expected is None or expected not in row["top"]["dense"][:10]
                else row["top"]["dense"].index(expected) + 1,
                "lexical": None
                if expected is None or expected not in row["top"]["lexical"][:10]
                else row["top"]["lexical"].index(expected) + 1,
            }
        )
    controls = artefact["controls"]
    out.extend(
        [
            "Contrôles de cohérence et question non satisfiable :",
            "",
            f"- α = 0 reproduit le lexical seul sur "
            f"{controls['alpha_0_equals_lexical']}/{controls['questions']} questions ;",
            f"- α = 1 reproduit le dense seul sur "
            f"{controls['alpha_1_equals_dense']}/{controls['questions']} questions ;",
            f"- {UNSATISFIABLE} est hors de toute moyenne ; rang de son attendu "
            f"`{unsatisfiable_rows[0]['expected']}` (version ABROGE_DIFF, hors du "
            f"périmètre `textes_vigueur`, mais présent comme distracteur) : "
            f"dense {unsatisfiable_rows[0]['dense'] or '—'}, "
            f"lexical {unsatisfiable_rows[0]['lexical'] or '—'}.",
            "",
        ]
    )
    rejected = artefact["decision"].get("rejected") or []
    out.extend(
        [
            "Configurations écartées par la règle de décision :",
            "",
            markdown_table(
                [
                    {
                        "method": labels.get(entry["method"], entry["method"]),
                        "low": entry["low"],
                        "high": entry["high"],
                        "reason": entry["reason"],
                    }
                    for entry in rejected
                ],
                [
                    ("method", "Mode"),
                    ("low", "borne basse"),
                    ("high", "borne haute"),
                    ("reason", "Motif"),
                ],
            ),
            "",
        ]
    )
    return out


def build_11(
    raw: Path,
    questions: Mapping[str, dict],
    commit: str,
    date: str,
    machine: str,
    vectors: Path,
    measures: Path,
) -> tuple[dict, list[str]]:
    """Recomputes MES-11 and returns its artefact and the report tables."""
    documents = {}
    for structure in STRUCTURES:
        path = raw / f"analyze-{structure}.json"
        if not path.is_file():
            raise SystemExit(f"missing raw document {path.name}")
        documents[structure] = json.loads(path.read_text(encoding="utf-8"))

    rows: list[dict] = []
    anchor = {"reproduced": 0, "compared": 0}
    probes_by_structure: dict[str, list[dict]] = {}
    stats: dict[str, dict] = {}
    differences: list[dict] = []
    for structure, document in documents.items():
        anchor["reproduced"] += document["anchor"]["v0_replica_equals_engine_documents"]
        anchor["compared"] += document["anchor"]["questions"]
        for entry in document["variants"]:
            current = stats.setdefault(entry["variant"], {"passages": 0, "tokens": 0,
                                                           "differing": 0})
            current["passages"] += entry["stats"]["passages"]
            current["tokens"] += entry["stats"]["tokens"]
            current["differing"] += entry["stats"]["passages_differing_from_v0"]
        differences.extend(
            {**entry, "structure": structure} for entry in document["differences"]
        )
        probes_by_structure[structure] = document["probes"]
        for row in document["questions"]:
            rows.append(
                row
                | {
                    "structure": structure,
                    "corpus": questions[row["id"]]["corpus"],
                    "satisfiable": questions[row["id"]]["satisfiable"],
                }
            )

    # Probes target documents of the `articles` index: measuring them on the
    # other five indexes repeats the same tokenization, so the artefact checks
    # that stability once instead of listing the probes six times.
    probes = probes_by_structure.get("articles", [])
    probe_stability = {"compared": 0, "identical": 0}
    for probe in probes:
        for other in probes_by_structure.values():
            match = next(
                (entry for entry in other if entry["name"] == probe["name"]), None
            )
            if match is None:
                continue
            for variant in VARIANTS:
                probe_stability["compared"] += 1
                if match["tokens"][variant] == probe["tokens"][variant]:
                    probe_stability["identical"] += 1

    measured = [row for row in rows if row["id"] != UNSATISFIABLE]
    # Decision 5 defines the judged questions as the union of the questions that
    # hold an elision and those that hold an identifier. The complement (all 56
    # satisfiable questions) is reported next to it, and never replaces it.
    series = [row for row in measured if row["elision"] or row["identifier"]]
    for row in measured:
        row["top"] = {variant: row["lexical"][variant] for variant in VARIANTS}
    summary = {variant: summarise(series, variant) for variant in VARIANTS}
    intervals = {
        variant: bootstrap(
            per_question(series, variant),
            per_question(series, "V0"),
        )
        for variant in VARIANTS
        if variant != "V0"
    }
    complement_summary = {variant: summarise(measured, variant) for variant in VARIANTS}
    complement_intervals = {
        variant: bootstrap(
            per_question(measured, variant),
            per_question(measured, "V0"),
        )
        for variant in VARIANTS
        if variant != "V0"
    }

    # Secondary series: the same rule over the `rrf` document top-10 of each
    # variant, recomputed by the harness from the engine's dense ranking and the
    # replica's lexical ranking.
    for row in measured:
        row["top"] = {variant: row["rrf"][variant] for variant in VARIANTS}
    fused_summary = {variant: summarise(series, variant) for variant in VARIANTS}
    fused_intervals = {
        variant: bootstrap(
            per_question(series, variant),
            per_question(series, "V0"),
        )
        for variant in VARIANTS
        if variant != "V0"
    }
    fused_complement = {
        variant: summarise(measured, variant) for variant in VARIANTS
    }

    by_corpus: dict[str, dict[str, dict]] = {}
    for corpus in sorted({row["corpus"] for row in series}):
        subset = [row for row in series if row["corpus"] == corpus]
        by_corpus[corpus] = {
            variant: {
                "lexical": summarise(
                    [row | {"top": row["lexical"]} for row in subset], variant
                ),
                "rrf": summarise([row | {"top": row["rrf"]} for row in subset], variant),
            }
            for variant in VARIANTS
        }

    elisions = [row["id"] for row in measured if row["elision"]]
    identifiers = [row["id"] for row in measured if row["identifier"]]
    without_expected = [probe["name"] for probe in probes if probe["expected"] is None]
    probe_rows = []
    for probe in probes:
        probe_rows.append(
            {
                "name": probe["name"],
                "text": probe["text"],
                "expected": probe["expected"],
                "lexical_rank": probe["lexical_rank"],
                "lexical": probe["lexical"],
                "lexical_count": {
                    variant: len(probe["lexical"][variant]) for variant in VARIANTS
                },
                "rrf_top3": {
                    variant: (
                        None
                        if probe["expected"] is None
                        else probe["rrf"][variant][:3].count(probe["expected"]) > 0
                    )
                    for variant in VARIANTS
                },
                "tokens": probe["tokens"],
                "rrf": probe["rrf"],
            }
        )
    probes_pass = {
        variant: all(
            probe["rrf"][variant][:3].count(probe["expected"]) > 0
            for probe in probes
            if probe["expected"] is not None
        )
        for variant in VARIANTS
    }

    decision = decide(
        {variant: summary[variant] for variant in VARIANTS},
        {variant: intervals[variant] for variant in VARIANTS if variant != "V0"},
        reference="V0",
    )
    decision["probes_top3"] = probes_pass

    artefact = {
        "measure": "MES-11",
        "protocol": "ADR-0092",
        "date": date,
        "commit": commit,
        "machine": machine,
        "query_vectors": {
            "path": vectors.relative_to(ROOT).as_posix(),
            "sha256": sha256(vectors),
        },
        "raw": {
            path.relative_to(ROOT).as_posix(): sha256(path)
            for path in sorted(raw.glob("analyze-*.json"))
        },
        "variants": {
            "V0": "SimpleTokenizer, RemoveLongFilter(40), LowerCaser, AsciiFoldingFilter, "
            "StopWordFilter(fr), Stemmer(fr)",
            "V1": "V0 plus an elision filter",
            "V2": "V0 with identifiers kept as one token (hyphen kept)",
            "V3": "V1 and V2",
        },
        "passages": sum(entry["stats"]["passages"] for document in documents.values()
                        for entry in document["variants"] if entry["variant"] == "V0"),
        "token_statistics": stats,
        "anchor": anchor,
        "questions": {
            "averaged": len(series),
            "definition": "questions holding an elision or an identifier (union)",
            "complement": len(measured),
            "excluded": [UNSATISFIABLE],
        },
        "elisions": {"count": len(elisions), "ids": elisions},
        "identifiers": {"count": len(identifiers), "ids": identifiers},
        "lexical": {
            "summary": summary,
            "bootstrap": intervals,
            "by_corpus": by_corpus,
            "complement": {
                "summary": complement_summary,
                "bootstrap": complement_intervals,
            },
        },
        "rrf": {
            "summary": fused_summary,
            "bootstrap": fused_intervals,
            "complement": fused_complement,
        },
        "probes": probe_rows,
        "probes_top3": probes_pass,
        "probes_without_expected": without_expected,
        "probe_token_stability": probe_stability,
        "differences": differences,
        "decision": decision,
    }

    tables = render_11(artefact)
    if measures is not None:
        (measures / f"{SLUG_11}.{date}.json").write_text(
            json.dumps(artefact, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
        )
    return artefact, tables


def render_11(artefact: Mapping[str, object]) -> list[str]:
    """The Markdown tables of MES-11, in the order of the report."""
    summary = artefact["lexical"]["summary"]
    intervals = artefact["lexical"]["bootstrap"]
    fused = artefact["rrf"]["summary"]
    rows = []
    for variant in VARIANTS:
        interval = intervals.get(variant)
        rows.append(
            {
                "variant": variant,
                "recall": summary[variant]["recall"],
                "mrr": summary[variant]["mrr"],
                "median": summary[variant]["median_rank"],
                "fused": fused[variant]["recall"],
                "difference": None if interval is None else interval["difference"],
                "ci": "—"
                if interval is None
                else f"[{interval['low']:+.4f} ; {interval['high']:+.4f}]",
            }
        )
    out = [
        "Branche lexicale (série principale) et `rrf` (série secondaire), "
        f"{artefact['questions']['averaged']} questions de la série principale "
        "(union élision ∪ identifiant) :",
        "",
        markdown_table(
            rows,
            [
                ("variant", "Variante"),
                ("recall", "recall@10 lexical"),
                ("mrr", "MRR@10 lexical"),
                ("median", "rang médian"),
                ("fused", "recall@10 `rrf`"),
                ("difference", "écart à V0"),
                ("ci", "IC 95 %"),
            ],
        ),
        "",
    ]
    complement = artefact["lexical"]["complement"]
    complement_fused = artefact["rrf"]["complement"]
    complement_rows = [
        {
            "variant": variant,
            "recall": complement["summary"][variant]["recall"],
            "mrr": complement["summary"][variant]["mrr"],
            "median": complement["summary"][variant]["median_rank"],
            "fused": complement_fused[variant]["recall"],
            "difference": None
            if variant == "V0"
            else complement["bootstrap"][variant]["difference"],
            "ci": "—"
            if variant == "V0"
            else f"[{complement['bootstrap'][variant]['low']:+.4f} ; "
            f"{complement['bootstrap'][variant]['high']:+.4f}]",
        }
        for variant in VARIANTS
    ]
    out.extend(
        [
            f"Complément déclaré, {artefact['questions']['complement']} questions "
            "satisfiables (les 9 questions sans élision ni identifiant sont "
            "incluses) :",
            "",
            markdown_table(
                complement_rows,
                [
                    ("variant", "Variante"),
                    ("recall", "recall@10 lexical"),
                    ("mrr", "MRR@10 lexical"),
                    ("median", "rang médian"),
                    ("fused", "recall@10 `rrf`"),
                    ("difference", "écart à V0"),
                    ("ci", "IC 95 %"),
                ],
            ),
            "",
        ]
    )
    probes = []
    for probe in artefact["probes"]:
        probes.append(
            {
                "name": probe["name"],
                "text": probe["text"],
                "expected": probe["expected"] or "—",
                "v0": probe["lexical_rank"]["V0"],
                "v1": probe["lexical_rank"]["V1"],
                "v2": probe["lexical_rank"]["V2"],
                "v3": probe["lexical_rank"]["V3"],
            }
        )
    out.extend(
        [
            "Sondes déclarées, rang du document attendu en lexical "
            "(index `articles`, seul index qui porte les attendus). "
            f"Sans attendu : {', '.join(artefact['probes_without_expected'])} ; "
            "`probes_top3` ne repose que sur les autres :",
            "",
            markdown_table(
                probes,
                [
                    ("name", "Sonde"),
                    ("text", "Texte"),
                    ("expected", "Attendu"),
                    ("v0", "V0"),
                    ("v1", "V1"),
                    ("v2", "V2"),
                    ("v3", "V3"),
                ],
            ),
            "",
        ]
    )
    statistic_rows = [
        {
            "variant": variant,
            "passages": artefact["token_statistics"][variant]["passages"],
            "differing": artefact["token_statistics"][variant]["differing"],
            "tokens": artefact["token_statistics"][variant]["tokens"],
        }
        for variant in VARIANTS
    ]
    out.extend(
        [
            f"Tokenisation des {artefact['passages']} passages des six index :",
            "",
            markdown_table(
                statistic_rows,
                [
                    ("variant", "Variante"),
                    ("passages", "Passages"),
                    ("differing", "Passages différents de V0"),
                    ("tokens", "Jetons"),
                ],
            ),
            "",
            "Sondes dans le top 3 en `rrf` : "
            + ", ".join(
                f"{variant} {'oui' if passed else 'non'}"
                for variant, passed in artefact["probes_top3"].items()
            )
            + ".",
            "",
            "Stabilité de la tokenisation des sondes entre les six index : "
            f"{artefact['probe_token_stability']['identical']}"
            f"/{artefact['probe_token_stability']['compared']} couples "
            "(sonde, variante) identiques.",
            "",
            f"{len(artefact['differences'])} exemples de différences de "
            "tokenisation sont consignés dans l'artefact.",
            "",
        ]
    )
    return out


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--raw", type=Path, default=DEFAULT_RAW)
    parser.add_argument("--measures", type=Path, default=DEFAULT_MEASURES)
    parser.add_argument(
        "--queries",
        type=Path,
        default=ROOT / "data" / "legifacile-sample" / "queries.jsonl",
    )
    parser.add_argument(
        "--vectors",
        type=Path,
        default=ROOT / "data" / "mes10" / "query-vectors-2026-10-28.json",
    )
    parser.add_argument("--commit", required=True)
    parser.add_argument("--date", required=True)
    parser.add_argument("--machine", required=True)
    parser.add_argument("--write", action="store_true")
    options = parser.parse_args()

    questions = load_questions(options.queries)
    measures = options.measures if options.write else None
    ten, tables_10 = build_10(
        options.raw, questions, options.commit, options.date, options.machine,
        options.vectors, measures,
    )
    eleven, tables_11 = build_11(
        options.raw, questions, options.commit, options.date, options.machine,
        options.vectors, measures,
    )
    print("\n".join(tables_10))
    print("\n".join(tables_11))
    print(
        f"MES-10: recommande {ten['decision']['recommended']} "
        f"({ten['decision']['reason']})",
        flush=True,
    )
    print(
        f"MES-11: recommande {eleven['decision']['recommended']}, "
        f"sondes top 3 : {eleven['probes_top3']}",
        flush=True,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
