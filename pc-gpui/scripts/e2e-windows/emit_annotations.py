"""把 e2e 结果以 GitHub Actions 注解输出（可经 check-runs annotations API 取回，产物下载不可用时用）。

    python emit_annotations.py failures <out>                 # 每个 FAIL 一条 error 注解：用例输出的最后 ~3800 字符
    python emit_annotations.py images <out> --skip 0 --max 20 # <out>/emit/*.png 的 base64 分片（notice / warning）

GitHub 限制：注解正文最多 4096 字符；每个步骤每种级别最多 10 条、每个 job 最多 50 条。所以图片按 3900 字符分片，
一个步骤先发 10 条 notice 再发 10 条 warning；分片多时由多个步骤（--skip）接着发。
标题格式 `IMG <名字> <k>/<n>`，取回：`gh api repos/<o>/<r>/check-runs/<job_id>/annotations`，按标题拼接后 base64 解码。
"""
from __future__ import annotations

import argparse
import base64
import json
import sys
from pathlib import Path

CHUNK = 3900


def esc(s: str) -> str:
    return s.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")


def esc_prop(s: str) -> str:
    return esc(s).replace(":", "%3A").replace(",", "%2C")


def failures(out: Path) -> int:
    results = json.loads((out / "results.json").read_text(encoding="utf-8")) if (out / "results.json").exists() else []
    fails = [r for r in results if r["status"] == "FAIL"]
    counts = {s: sum(1 for r in results if r["status"] == s) for s in ("PASS", "FAIL", "SKIP")}
    summary = f"PASS {counts['PASS']} / FAIL {counts['FAIL']} / SKIP {counts['SKIP']}; FAIL: " + \
        ", ".join(r["id"] for r in fails)
    print(f"::warning title={esc_prop('E2E SUMMARY')}::{esc(summary)}")
    for r in fails[:10]:
        detail_file = out / f"fail-{r['id']}.txt"
        body = r["detail"] + "\n---\n"
        if detail_file.exists():
            text = detail_file.read_text(encoding="utf-8", errors="replace")
            body += text[-(3800 - len(body)):] if len(body) < 3800 else ""
        print(f"::error title={esc_prop('E2E FAIL ' + r['id'] + ' ' + r['title'][:60])}::{esc(body[:4000])}")
    return 0


def images(out: Path, skip: int, limit: int) -> int:
    files = sorted((out / "emit").glob("*.png")) if (out / "emit").exists() else []
    chunks = []
    for f in files:
        b64 = base64.b64encode(f.read_bytes()).decode()
        parts = [b64[i:i + CHUNK] for i in range(0, len(b64), CHUNK)]
        for k, p in enumerate(parts):
            chunks.append((f.stem, k + 1, len(parts), p))
    print(f"{len(files)} 张图，{len(chunks)} 个分片；本步发第 {skip + 1}~{min(len(chunks), skip + limit)} 个")
    for i, (name, k, n, p) in enumerate(chunks[skip:skip + limit]):
        level = "notice" if i < 10 else "warning"
        print(f"::{level} title={esc_prop(f'IMG {name} {k}/{n}')}::{p}")
    if len(chunks) > skip + limit:
        print(f"还有 {len(chunks) - skip - limit} 个分片没有发出（超出注解配额）")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("mode", choices=["failures", "images"])
    ap.add_argument("out")
    ap.add_argument("--skip", type=int, default=0)
    ap.add_argument("--max", type=int, default=20)
    a = ap.parse_args()
    out = Path(a.out)
    return failures(out) if a.mode == "failures" else images(out, a.skip, a.max)


if __name__ == "__main__":
    sys.exit(main())
