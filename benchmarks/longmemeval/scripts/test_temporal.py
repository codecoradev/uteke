#!/usr/bin/env python3
"""Unit tests for temporal.py (#1119).

Run: python3 -m pytest test_temporal.py -v   (or python3 test_temporal.py)
"""
import sys
from datetime import date
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from temporal import (parse_temporal, parse_question_date, in_window,
                      boost_ranking, RRF_K)


# ---------- parse_question_date ----------

def test_qdate_standard():
    assert parse_question_date("2023/03/22 (Wed) 21:49") == date(2023, 3, 22)

def test_qdate_none():
    assert parse_question_date(None) is None
    assert parse_question_date("") is None
    assert parse_question_date("no date here") is None


# ---------- parse_temporal: resolver coverage ----------

Q = date(2023, 3, 22)

def test_between():
    w = parse_temporal("What did I do between February and April?", Q)
    assert w[2] == "between"
    assert w[0] == date(2023, 2, 1)
    assert w[1] == date(2023, 4, 30)

def test_before():
    w = parse_temporal("What did I buy before January?", Q)
    assert w[2] == "before"
    assert w[0] is None and w[1] == date(2023, 1, 31)

def test_after():
    w = parse_temporal("What changed after March?", Q)
    assert w[2] == "after/since"
    assert w[0] == date(2023, 3, 1) and w[1] is None

def test_since_with_year():
    w = parse_temporal("since May 2022", Q)
    assert w[0] == date(2022, 5, 1) and w[1] is None

def test_in_month():
    w = parse_temporal("What did I eat in February?", Q)
    assert w[2] == "in-month"
    assert w[0] == date(2023, 2, 1) and w[1] == date(2023, 2, 28)

def test_in_month_wrap_year():
    # 'May' asked in March 2023 -> 2022-05 (haystack year precedes question)
    w = parse_temporal("in May", Q)
    assert w[0] == date(2022, 5, 1)

def test_last_n_months():
    w = parse_temporal("What did I cook in the last 3 months?", Q)
    assert w[2] == "last-N"
    assert w[0] == date(2022, 12, 22) and w[1] == Q

def test_last_month():
    w = parse_temporal("last month", Q)
    assert w[2] == "last-month"
    assert w[0] == date(2023, 2, 1) and w[1] == date(2023, 2, 28)

def test_last_month_january_edge():
    w = parse_temporal("last month", date(2023, 1, 15))
    assert w[0] == date(2022, 12, 1) and w[1] == date(2022, 12, 31)

def test_this_month():
    w = parse_temporal("this month", Q)
    assert w[0] == date(2023, 3, 1)
    assert w[1] == Q  # window extends through question date itself

def test_this_year():
    w = parse_temporal("this year", Q)
    assert w[0] == date(2023, 1, 1) and w[1] == Q

def test_no_temporal():
    assert parse_temporal("What is my favorite sushi?", Q) is None
    # Month name used as a proper noun must NOT parse (strict patterns
    # require a preposition, e.g. "in January", not "January Jones").
    assert parse_temporal("January Jones is an actress", Q) is None
    assert parse_temporal("", Q) is None
    assert parse_temporal("last month", None) is None


# ---------- in_window ----------

def test_in_window_basic():
    w = (date(2023, 2, 1), date(2023, 2, 28), "t")
    assert in_window(date(2023, 2, 15), w)
    assert not in_window(date(2023, 3, 1), w)

def test_in_window_open_bounds():
    w = (None, date(2023, 1, 31), "t")
    assert in_window(date(2020, 6, 1), w)
    assert not in_window(date(2023, 2, 1), w)


# ---------- boost_ranking ----------

def test_boost_lifts_in_window_session():
    # session at rank 9 (RRF 1/69 = 0.01449) with boost 0.002 -> 0.01649
    # rank 5 (1/65 = 0.01538) without boost stays below it? 0.01538 < 0.01649 yes
    sids = [f"s{i}" for i in range(1, 16)]
    dates = {f"s{i}": date(2023, 2, 10) if i == 9 else date(2022, 6, 1) for i in range(1, 16)}
    w = (date(2023, 2, 1), date(3, 1, 1) and date(2023, 2, 28), "t")
    out = boost_ranking(sids, dates, w)
    assert out[0] == "s9", "in-window session at rank 9 should lift to top-5 (and here to #1 vs out-of-window ranks 2-4)"

def test_boost_no_window_is_noop():
    sids = [f"s{i}" for i in range(1, 10)]
    out = boost_ranking(sids, {}, None)
    assert out == sids

def test_boost_without_dates_is_noop():
    sids = [f"s{i}" for i in range(1, 10)]
    w = (date(2023, 2, 1), date(2023, 2, 28), "t")
    out = boost_ranking(sids, {}, w)
    assert out == sids

def test_boost_preserves_order_within_window():
    # two in-window sessions keep their recall order relative to each other
    sids = [f"s{i}" for i in range(1, 12)]
    dates = {sid: date(2023, 2, 5) for sid in sids}
    w = (date(2023, 2, 1), date(2023, 2, 28), "t")
    out = boost_ranking(sids, dates, w)
    assert out == sids

def test_boost_stable_semantic_top_ranked_not_demoted():
    # rank-1 session, in-window, must stay #1 even if many others boosted
    sids = [f"s{i}" for i in range(1, 26)]
    dates = {sid: date(2023, 2, 5) for sid in sids}
    w = (date(2023, 2, 1), date(2023, 2, 28), "t")
    out = boost_ranking(sids, dates, w)
    assert out[0] == "s1"

def test_rank_math():
    # Boundary property for recall@5: an in-window rank-15 must beat an
    # out-of-window rank-5 (membership crossing), while a rank-1 out-of-window
    # still beats an in-window rank-15 (head of ranking stays semantic).
    r15_boosted = 1 / (RRF_K + 15) + 0.0022
    r5 = 1 / (RRF_K + 5)
    r1 = 1 / (RRF_K + 1)
    assert r15_boosted > r5, "in-window rank-15 must cross the rank-5 boundary"
    assert r1 > r15_boosted, "rank-1 must stay above a boosted rank-15"


if __name__ == "__main__":
    fails = 0
    for name, fn in [(n, f) for n, f in sorted(globals().items()) if n.startswith("test_") and callable(f)]:
        try:
            fn()
            print(f"PASS {name}")
        except AssertionError as e:
            fails += 1
            print(f"FAIL {name}: {e}")
    print(f"\n{fails} failure(s)")
    sys.exit(1 if fails else 0)
