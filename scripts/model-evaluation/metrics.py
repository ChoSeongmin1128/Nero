import unicodedata
from collections import Counter

MAX_DP_CELLS = 2_000_000


def _edits(reference, hypothesis):
    n, m = len(reference), len(hypothesis)
    if (n + 1) * (m + 1) > MAX_DP_CELLS:
        try:
            from rapidfuzz.distance import Levenshtein
        except ImportError:
            return None
        counts = Counter(op.tag for op in Levenshtein.editops(reference, hypothesis))
        return {"substitutions": counts["replace"], "deletions": counts["delete"], "insertions": counts["insert"], "reference_length": n}
    previous = [(j, 0, 0, j) for j in range(m + 1)]
    for i, a in enumerate(reference, 1):
        current = [(i, 0, i, 0)]
        for j, b in enumerate(hypothesis, 1):
            if a == b:
                current.append(previous[j - 1])
            else:
                cost, s, d, ins = previous[j - 1]
                replace = (cost + 1, s + 1, d, ins)
                cost, s, d, ins = previous[j]
                delete = (cost + 1, s, d + 1, ins)
                cost, s, d, ins = current[j - 1]
                insert = (cost + 1, s, d, ins + 1)
                current.append(min((replace, delete, insert), key=lambda x: x[0]))
        previous = current
    _, s, d, ins = previous[-1]
    return {"substitutions": s, "deletions": d, "insertions": ins, "reference_length": n}


def _rate(reference, hypothesis):
    counts = _edits(reference, hypothesis)
    if counts is None:
        return {"status": "unsupported", "reason": "input exceeds bounded exact DP; install RapidFuzz for exact edit operations"}
    denominator = counts["reference_length"]
    errors = counts["substitutions"] + counts["deletions"] + counts["insertions"]
    return {"status": "scored" if denominator else "not_scored", "value": errors / denominator if denominator else None, **counts}


def text_metrics(reference, hypothesis):
    reference = unicodedata.normalize("NFC", reference)
    hypothesis = unicodedata.normalize("NFC", hypothesis)
    normalized_reference = " ".join(reference.split())
    normalized_hypothesis = " ".join(hypothesis.split())
    return {
        "policy": "NFC; strict CER includes all characters; normalized CER collapses Unicode whitespace to one space and trims ends; WER uses whitespace-separated words; punctuation and case retained",
        "strict_cer": _rate(reference, hypothesis),
        "normalized_cer": _rate(normalized_reference, normalized_hypothesis),
        "normalized_wer": _rate(normalized_reference.split(), normalized_hypothesis.split()),
    }


def correction_metrics(reference, baseline, corrected):
    before = text_metrics(reference, baseline)
    after = text_metrics(reference, corrected)
    old = before["strict_cer"].get("value")
    new = after["strict_cer"].get("value")
    return {"baseline": before, "after": after, "strict_cer_delta": new - old if old is not None and new is not None else None, "unchanged": baseline == corrected}


def entity_metrics(reference, hypothesis, evaluation_labels):
    reference = unicodedata.normalize("NFC", reference)
    hypothesis = unicodedata.normalize("NFC", hypothesis)
    labels = list(dict.fromkeys(unicodedata.normalize("NFC", label) for label in evaluation_labels))
    if any(not label for label in labels):
        raise ValueError("evaluation labels must be nonempty")
    rows = [{"label": label, "spoken_reference": label in reference, "present_in_hypothesis": label in hypothesis} for label in labels]
    spoken = [row for row in rows if row["spoken_reference"]]
    hits = sum(row["present_in_hypothesis"] for row in spoken)
    return {"policy": "explicit evaluation labels only; exact NFC substring presence; labels absent from reference cannot count as hits", "labels": rows, "hits": hits, "reference_labels": len(spoken), "accuracy": hits / len(spoken) if spoken else None, "hallucinated_labels": [row["label"] for row in rows if not row["spoken_reference"] and row["present_in_hypothesis"]]}


def _assignment(weights):
    n = len(weights)
    if not n:
        return []
    m = len(weights[0]) if weights else 0
    size = max(n, m)
    maximum = max((max(row, default=0) for row in weights), default=0)
    cost = [[maximum - (weights[i][j] if i < n and j < m else 0) for j in range(size)] for i in range(size)]
    u = [0.0] * (size + 1)
    v = [0.0] * (size + 1)
    p = [0] * (size + 1)
    way = [0] * (size + 1)
    for i in range(1, size + 1):
        p[0] = i
        j0 = 0
        minv = [float("inf")] * (size + 1)
        used = [False] * (size + 1)
        while True:
            used[j0] = True
            i0 = p[j0]
            delta = float("inf")
            for j in range(1, size + 1):
                if not used[j]:
                    cur = cost[i0 - 1][j - 1] - u[i0] - v[j]
                    if cur < minv[j]:
                        minv[j], way[j] = cur, j0
                    if minv[j] < delta:
                        delta, j1 = minv[j], j
            for j in range(size + 1):
                if used[j]:
                    u[p[j]] += delta
                    v[j] -= delta
                else:
                    minv[j] -= delta
            j0 = j1
            if p[j0] == 0:
                break
        while True:
            j1 = way[j0]
            p[j0] = p[j1]
            j0 = j1
            if j0 == 0:
                break
    return [(p[j] - 1, j - 1) for j in range(1, size + 1) if p[j] and p[j] <= n and j <= m]


def diarization_metrics(reference_segments, hypothesis_segments, collar=0.0, overlap_policy="include", output_available=None):
    import math
    if not math.isfinite(collar) or collar < 0 or overlap_policy not in ("include", "exclude"):
        raise ValueError("invalid collar or overlap policy")
    for segment in [*reference_segments, *hypothesis_segments]:
        if not isinstance(segment.get("speaker"), str) or not segment["speaker"] or not all(isinstance(segment.get(k), (int, float)) and math.isfinite(segment[k]) for k in ("start", "end")) or segment["start"] < 0 or segment["end"] <= segment["start"]:
            raise ValueError("invalid speaker segment")
    refs = sorted({s["speaker"] for s in reference_segments})
    hyps = sorted({s["speaker"] for s in hypothesis_segments})
    boundaries = {v for s in [*reference_segments, *hypothesis_segments] for v in (s["start"], s["end"])}
    collars = [(max(0, v - collar), v + collar) for s in reference_segments for v in (s["start"], s["end"]) if collar]
    boundaries.update(v for pair in collars for v in pair)
    points = sorted(boundaries)
    weights = [[0.0 for _ in hyps] for _ in refs]
    regions = []
    for start, end in zip(points, points[1:]):
        mid = (start + end) / 2
        if any(a <= mid < b for a, b in collars):
            continue
        r = {s["speaker"] for s in reference_segments if s["start"] <= mid < s["end"]}
        h = {s["speaker"] for s in hypothesis_segments if s["start"] <= mid < s["end"]}
        if overlap_policy == "exclude" and len(r) > 1:
            continue
        duration = end - start
        regions.append((duration, r, h))
        for i, ref in enumerate(refs):
            for j, hyp in enumerate(hyps):
                if ref in r and hyp in h:
                    weights[i][j] += duration
    mapping = {hyps[j]: refs[i] for i, j in _assignment(weights)}
    false_alarm = miss = confusion = total = 0.0
    for duration, r, h in regions:
        matches = sum(mapping.get(speaker) in r for speaker in h)
        total += duration * len(r)
        false_alarm += duration * max(0, len(h) - len(r))
        miss += duration * max(0, len(r) - len(h))
        confusion += duration * (min(len(r), len(h)) - matches)
    scored = bool(refs and total and output_available is not False and (hyps or output_available is True))
    return {"status": "scored" if scored else "not_scored", "der": (false_alarm + miss + confusion) / total if scored else None, "false_alarm_seconds": false_alarm, "miss_seconds": miss, "confusion_seconds": confusion, "reference_speaker_seconds": total, "mapping": mapping, "policy": {"collar_seconds_each_side_of_reference_boundary": collar, "overlap": overlap_policy, "mapping": "global maximum overlap", "denominator": "reference speaker-seconds", "missing_reference_or_unavailable_output": "not_scored; explicit available empty output scores missed speech"}}
