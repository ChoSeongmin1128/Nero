import csv
import html
import json
import os
from pathlib import Path


FIELDS = ("case_id", "experiment", "track", "model", "hardware", "decoding", "context", "status", "timed_words", "full_input_completed", "execution", "metrics")


def write_report(results, output_dir):
    rows = list(results)
    for row in rows:
        missing = [key for key in ("case_id", "track", "model", "hardware", "decoding", "status", "metrics") if key not in row]
        if missing:
            raise ValueError("missing report fields: " + ", ".join(missing))
        if row["status"] not in ("scored", "failed", "unsupported", "unqualified", "not_scored"):
            raise ValueError("invalid report status")
    rows.sort(key=lambda r: tuple(str(r.get(key, "")) for key in ("case_id", "track", "model", "hardware", "decoding", "status")))
    directory = Path(output_dir)
    directory.mkdir(parents=True, exist_ok=True, mode=0o700)
    os.chmod(directory, 0o700)
    json_data = json.dumps(rows, ensure_ascii=False, indent=2, allow_nan=False) + "\n"
    def private_write(name, content):
        path = directory / name
        fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        with os.fdopen(fd, "w", encoding="utf-8", newline="") as handle:
            os.fchmod(handle.fileno(), 0o600)
            handle.write(content)
    private_write("report.json", json_data)
    import io
    output = io.StringIO(newline="")
    writer = csv.DictWriter(output, fieldnames=FIELDS, extrasaction="ignore")
    writer.writeheader()
    for row in rows:
        item = {key: row.get(key) for key in FIELDS}
        for key in ("hardware", "decoding", "execution", "metrics"):
            item[key] = json.dumps(item[key], ensure_ascii=False, sort_keys=True, allow_nan=False)
        writer.writerow(item)
    private_write("report.csv", output.getvalue())
    def cell(value):
        if isinstance(value, (dict, list)):
            value = json.dumps(value, ensure_ascii=False, sort_keys=True, allow_nan=False)
        elif value is None:
            value = "unknown"
        return html.escape(str(value), quote=True)
    body = "".join("<tr>" + "".join("<td>" + cell(row.get(field)) + "</td>" for field in FIELDS) + "</tr>" for row in rows)
    headers = "".join("<th>" + cell(field) + "</th>" for field in FIELDS)
    page = '''<!doctype html><html lang="ko"><meta charset="utf-8"><meta name="viewport" content="width=device-width"><title>모델 평가 결과</title><style>body{font:14px system-ui;margin:2rem;color:#202324}table{border-collapse:collapse;width:100%}td,th{border:1px solid #ccd1d2;padding:.5rem;text-align:left;vertical-align:top;overflow-wrap:anywhere}th{background:#eef2ef}input{padding:.5rem;width:24rem;max-width:90%}td:last-child{max-width:32rem}</style><h1>모델 평가 결과</h1><p>비공개 자료입니다. 동일한 사례와 평가 유형을 기준으로 비교하세요. 실패, 지원 불가, 자격 미달 결과는 순위에 포함하지 않습니다. 시간 정보 및 전체 입력 완료 여부가 비어 있으면 확인되지 않은 것입니다.</p><label>결과 필터 <input id="filter" type="search"></label><table><thead><tr>''' + headers + "</tr></thead><tbody>" + body + '''</tbody></table><script>document.querySelector('#filter').addEventListener('input',function(){const q=this.value.toLocaleLowerCase();for(const row of document.querySelectorAll('tbody tr'))row.hidden=!row.textContent.toLocaleLowerCase().includes(q)});</script></html>'''
    private_write("report.html", page)
    return {name: str(directory / name) for name in ("report.html", "report.json", "report.csv")}
