#!/usr/bin/env python3
"""Temporal expression parsing + date-window boost for LongMemEval (#1119).

Harness-side only: no uteke/Rust changes. When a question contains a temporal
expression ("last month", "before January", "in February", "between X and Y"),
we compute a date window relative to the question date and softly boost
retrieved sessions whose session date falls inside that window, before
Recall@k is computed. Boost is a small additive constant on an RRF-style rank
score, so it can lift an in-window session from rank ~10 into the top-5 but
never completely overrides semantic ranking.

Default OFF (--temporal flag in run_eval.py); when OFF or when no temporal
expression is parsed, output ordering is identical to baseline.
"""
import re
from datetime import date, timedelta

MONTHS = {
    "january": 1, "february": 2, "march": 3, "april": 4, "may": 5, "june": 6,
    "july": 7, "august": 8, "september": 9, "october": 10, "november": 11, "december": 12,
}
MONTH_RE = "|".join(MONTHS.keys())
UNITS = {"day": 1, "week": 7, "month": 30, "year": 365}

# Question date format in LongMemEval: "2023/03/22 (Wed) 21:49"
QDATE_RE = re.compile(r"(\d{4})/(\d{2})/(\d{2})")


def _days_in_month(year, month):
    if month == 12:
        return 31
    return (date(year, month + 1, 1) - timedelta(days=1)).day


def _month_start_abs(month, year):
    return date(year, month, 1)


def _month_end_abs(month, year):
    return date(year, month, _days_in_month(year, month))


def _resolve_year(month, year, q):
    """Bare month names resolve within the 12 months preceding the question.

    LongMemEval haystacks span roughly the year before the question date, so
    'January' asked in 2023/03 means 2023-01, while 'May' means 2022-05.
    """
    if year is not None:
        return year
    return q.year if month <= q.month else q.year - 1


def parse_question_date(qdate_str):
    """'2023/03/22 (Wed) 21:49' -> date(2023,3,22). None if unparseable."""
    m = QDATE_RE.search(qdate_str or "")
    if not m:
        return None
    return date(int(m.group(1)), int(m.group(2)), int(m.group(3)))


# Each pattern: (compiled regex, name, resolver(match, q) -> (lo, hi)).
# lo/hi are datetime.date or None for open-ended bounds. Ordered most
# specific first.
_RX_BETWEEN = re.compile(
    r"\bbetween\s+(" + MONTH_RE + r")(?:\s+(\d{4}))?\s+and\s+(" + MONTH_RE + r")(?:\s+(\d{4}))?\b",
    re.I,
)
_RX_BEFORE = re.compile(r"\bbefore\s+(" + MONTH_RE + r")(?:\s+(\d{4}))?\b", re.I)
_RX_AFTER = re.compile(r"\b(?:after|since)\s+(" + MONTH_RE + r")(?:\s+(\d{4}))?\b", re.I)
_RX_IN_MONTH = re.compile(r"\bin\s+(" + MONTH_RE + r")(?:\s+(\d{4}))?\b", re.I)
_RX_LAST_N = re.compile(r"\b(?:last|past|previous)\s+(\d+)\s+(day|week|month|year)s?\b", re.I)
_RX_N_AGO = re.compile(
    r"\b(\d+|one|two|three|four|five|six|seven|eight|nine|ten|eleven|twelve)"
    r"\s+(day|week|month)s?\s+ago\b",
    re.I,
)
_RX_LAST_WEEK = re.compile(r"\blast\s+week\b", re.I)
_RX_LAST_MONTH = re.compile(r"\blast\s+month\b", re.I)
_RX_THIS_MONTH = re.compile(r"\bthis\s+month\b", re.I)
_RX_THIS_YEAR = re.compile(r"\bthis\s+year\b", re.I)


def _res_between(m, q):
    m1, y1 = MONTHS[m.group(1).lower()], m.group(2)
    m2, y2 = MONTHS[m.group(3).lower()], m.group(4)
    lo = _month_start_abs(m1, _resolve_year(m1, int(y1) if y1 else None, q))
    hi_year = _resolve_year(m2, int(y2) if y2 else None, q)
    hi = _month_end_abs(m2, hi_year)
    if hi < lo:
        # Range crossing the question date (e.g. "between February and April"
        # asked in March): bare-month resolution can put hi a year before lo.
        # The range is stated in forward order, so bump hi into the next year.
        hi = _month_end_abs(m2, hi_year + 1)
    return (lo, hi)


def _res_before(m, q):
    mm, yy = MONTHS[m.group(1).lower()], m.group(2)
    hi = _month_end_abs(mm, _resolve_year(mm, int(yy) if yy else None, q))
    return (None, hi)


def _res_after(m, q):
    mm, yy = MONTHS[m.group(1).lower()], m.group(2)
    lo = _month_start_abs(mm, _resolve_year(mm, int(yy) if yy else None, q))
    return (lo, None)


def _res_in_month(m, q):
    mm, yy = MONTHS[m.group(1).lower()], m.group(2)
    y = _resolve_year(mm, int(yy) if yy else None, q)
    return (_month_start_abs(mm, y), _month_end_abs(mm, y))


def _res_last_n(m, q):
    n, unit = int(m.group(1)), m.group(2).lower()
    return (q - timedelta(days=n * UNITS[unit]), q)


def _res_last_month(m, q):
    first_this = _month_start_abs(q.month, q.year)
    last_prev = first_this - timedelta(days=1)  # final day of previous month
    return (_month_start_abs(last_prev.month, last_prev.year), last_prev)


_WORDS = {"one": 1, "two": 2, "three": 3, "four": 4, "five": 5, "six": 6,
          "seven": 7, "eight": 8, "nine": 9, "ten": 10, "eleven": 11, "twelve": 12}


def _res_n_ago(m, q):
    # "10 days ago" / "four weeks ago": the referenced event sits ~N units
    # before the question date. Window = [q - 1.5*span, q - 0.5*span] so the
    # nominal day lands mid-window with generous slack for fuzzy memories.
    g = m.group(1).lower()
    n = int(g) if g.isdigit() else _WORDS[g]
    span = n * UNITS[m.group(2).lower()]
    return (q - timedelta(days=int(1.5 * span)), q - timedelta(days=span // 2))


def _res_last_week(m, q):
    # Previous calendar week (Mon-Sun) relative to the question date.
    monday_this = q - timedelta(days=q.weekday())
    return (monday_this - timedelta(days=7), monday_this - timedelta(days=1))


def _res_this_month(m, q):
    return (_month_start_abs(q.month, q.year), q)


def _res_this_year(m, q):
    return (date(q.year, 1, 1), q)


_PATTERNS = [
    (_RX_BETWEEN, "between", _res_between),
    (_RX_BEFORE, "before", _res_before),
    (_RX_AFTER, "after/since", _res_after),
    (_RX_IN_MONTH, "in-month", _res_in_month),
    (_RX_N_AGO, "n-ago", _res_n_ago),
    (_RX_LAST_N, "last-N", _res_last_n),
    (_RX_LAST_WEEK, "last-week", _res_last_week),
    (_RX_LAST_MONTH, "last-month", _res_last_month),
    (_RX_THIS_MONTH, "this-month", _res_this_month),
    (_RX_THIS_YEAR, "this-year", _res_this_year),
]


def parse_temporal(question, qdate):
    """Parse the first matching temporal expression from a question.

    Returns (lo, hi, kind) — lo/hi are datetime.date or None (open bound) —
    or None when the question carries no recognized temporal expression.
    """
    if not question or qdate is None:
        return None
    for rx, name, fn in _PATTERNS:
        m = rx.search(question)
        if m:
            lo, hi = fn(m, qdate)
            return (lo, hi, name)
    return None


def in_window(d, window):
    """True if date d is within (lo, hi) inclusive; open ends are unbounded."""
    if not window:
        return False
    lo, hi, _ = window
    if lo and d < lo:
        return False
    if hi and d > hi:
        return False
    return True


RRF_K = 60  # standard RRF constant


def boost_ranking(session_ids, session_dates, window, boost=0.0022, rrf_k=RRF_K):
    """Re-rank deduped session ids with an additive temporal boost.

    session_ids:   recall rank order (index 0 = rank 1)
    session_dates: dict session_id -> datetime.date (metadata 'date'); missing ok
    window:        output of parse_temporal(), or None -> returned unchanged

    Rank r gets RRF score 1/(k+r); an in-window session gets +`boost`.
    Calibration goal (recall@5): an in-window session ranked 6..15 must pass
    an out-of-window session holding the rank-5 slot — 1/(k+15)+boost >
    1/(k+5) holds for boost=0.0022, k=60. Reordering *within* the top-5 is
    possible and harmless: recall@k membership only changes at the k/k+1
    boundary. Ties keep recall order (stable).
    """
    if not window or not session_ids:
        return session_ids
    scored = []
    for rank, sid in enumerate(session_ids, start=1):
        s = 1.0 / (rrf_k + rank)
        d = session_dates.get(sid)
        if d is not None and in_window(d, window):
            s += boost
        scored.append((s, -rank, sid))  # -rank: stable tie-break toward recall order
    scored.sort(key=lambda t: (t[0], t[1]), reverse=True)
    return [sid for _, _, sid in scored]
