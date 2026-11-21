"""packaging/av_scan.py with a fake network: no keys, no traffic."""

import datetime as dt
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "packaging"))

import av_scan  # noqa: E402
from av_scan import END, START, HybridAnalysis, Result, Target, VirusTotal  # noqa: E402


def _target(tmp_path, name="RealHiRes.exe", data=b"MZ" + b"\0" * 100):
    p = tmp_path / name
    p.write_bytes(data)
    return Target(str(p), name, av_scan.sha256_of(str(p)), len(data))


def _report(sha, kaspersky="undetected", flagged=()):
    results = {"Kaspersky": {"category": kaspersky, "engine_name": "Kaspersky",
                             "result": "HEUR:Trojan.Win32.Generic" if kaspersky == "malicious" else None}}
    for engine, label in flagged:
        results[engine] = {"category": "malicious", "engine_name": engine, "result": label}
    for i in range(60):
        results[f"Engine{i}"] = {"category": "undetected", "engine_name": f"Engine{i}", "result": None}
    mal = sum(1 for v in results.values() if v["category"] == "malicious")
    stats = {"malicious": mal, "suspicious": 0, "undetected": len(results) - mal, "harmless": 0,
             "type-unsupported": 3}
    return {"data": {"id": sha, "attributes": {"last_analysis_stats": stats, "last_analysis_results": results,
                                               "last_analysis_date": 1_900_000_000}}}


class FakeVT:
    """Answers like the VirusTotal API v3 for one unknown file."""

    def __init__(self, sha, report):
        self.sha, self.report, self.calls, self.uploaded, self.polls = sha, report, [], False, 0

    def __call__(self, method, url, headers=None, data=None, timeout=0):
        self.calls.append((method, url))
        assert headers["x-apikey"] == "k"
        if method == "GET" and url.endswith(f"/files/{self.sha}"):
            return (200, json.dumps(self.report).encode()) if self.uploaded else (404, b"{}")
        if method == "POST" and url.endswith("/files"):
            assert b'name="file"' in data and headers["content-type"].startswith("multipart/form-data")
            self.uploaded = True
            return 200, json.dumps({"data": {"type": "analysis", "id": "an-1"}}).encode()
        if method == "GET" and url.endswith("/analyses/an-1"):
            self.polls += 1
            state = "completed" if self.polls > 1 else "queued"
            return 200, json.dumps({"data": {"attributes": {"status": state}}}).encode()
        return 500, b""


def test_virustotal_uploads_unknown_file_and_reads_kaspersky(tmp_path):
    t = _target(tmp_path)
    fake = FakeVT(t.sha256, _report(t.sha256, flagged=[("Zillya", "Trojan.Stealer.Win64.2764")]))
    clock = [1_900_000_000.0]
    vt = VirusTotal("k", transport=fake, sleep=lambda s: clock.__setitem__(0, clock[0] + s), clock=lambda: clock[0])
    known = vt.report(t.sha256)
    assert known is None
    aid = vt.submit(t, known)
    assert aid == "an-1"
    vt.wait({t.sha256: aid}, deadline=clock[0] + 600)
    res, kas = VirusTotal.parse(t, vt.report(t.sha256))
    assert res.status == "detected" and res.summary == "1 / 62"
    assert res.detections == [("Zillya", "Trojan.Stealer.Win64.2764")]
    assert kas.status == "clean"
    # public API pacing: never more than 4 requests a minute
    assert clock[0] - 1_900_000_000.0 >= 15.5 * (len(fake.calls) - 1)


def test_known_recent_file_is_not_reanalysed(tmp_path):
    t = _target(tmp_path)
    vt = VirusTotal("k", transport=lambda *a, **k: (500, b""), sleep=lambda s: None, clock=lambda: 1_900_000_100)
    assert vt.submit(t, _report(t.sha256)) is None


def test_explanation_isolated_vs_serious(tmp_path):
    t = _target(tmp_path)
    vt, kas = VirusTotal.parse(t, _report(t.sha256, flagged=[("Zillya", "Trojan.Stealer.Win64.2764")]))
    text = av_scan.explain(t, {"VirusTotal": vt, "Kaspersky": kas}, "https://example.invalid/run/1")
    assert "Zillya (`Trojan.Stealer.Win64.2764`)" in text and "false positive" in text and t.sha256 in text
    vt, kas = VirusTotal.parse(t, _report(t.sha256, kaspersky="malicious"))
    text = av_scan.explain(t, {"VirusTotal": vt, "Kaspersky": kas}, "")
    assert "manual check" in text            # a major vendor flags it: no "false positive" claim
    vt, kas = VirusTotal.parse(t, _report(t.sha256))
    assert av_scan.explain(t, {"VirusTotal": vt, "Kaspersky": kas}, "") is None


def test_release_notes_section_is_replaced_not_duplicated(tmp_path):
    t = _target(tmp_path)
    vt, kas = VirusTotal.parse(t, _report(t.sha256))
    table = {t.sha256: {"VirusTotal": vt, "Kaspersky": kas,
                        "Hybrid Analysis": Result("not-configured", "open report", HybridAnalysis.gui(t.sha256))}}
    sec = av_scan.render("v1.8.1", [t], table, "", now=dt.datetime(2026, 11, 21, 15, 0),
                         configured={"VirusTotal": True, "Kaspersky": True, "Hybrid Analysis": False})
    assert "| `RealHiRes.exe` |" in sec and "Clean (0 / 61)" in sec
    assert "No engine reports" in sec and "Hybrid Analysis (no API key configured)" in sec
    body = av_scan.merge_notes("## What's new\n\ntext\n", sec)
    again = av_scan.merge_notes(body, sec.replace("15:00", "16:00"))
    assert again.count(START) == 1 and again.count(END) == 1 and "16:00" in again
    assert again.startswith("## What's new\n\ntext")


def test_main_without_keys_links_public_reports(tmp_path, monkeypatch):
    for k in ("VT_API_KEY", "KASPERSKY_OPENTIP_API_KEY", "HYBRID_ANALYSIS_API_KEY"):
        monkeypatch.delenv(k, raising=False)
    t = _target(tmp_path)
    notes_in, notes_out = tmp_path / "in.md", tmp_path / "out.md"
    notes_in.write_text("notes\n", encoding="utf-8")
    assert av_scan.main([t.path, "--tag", "v1", "--notes-in", str(notes_in), "--notes-out", str(notes_out),
                         "--inner", "RealHiRes.exe=RealHiRes-1-windows-x64.zip"]) == 0
    out = notes_out.read_text(encoding="utf-8")
    assert f"https://www.virustotal.com/gui/file/{t.sha256}" in out
    assert f"https://opentip.kaspersky.com/{t.sha256.upper()}/results?tab=upload" in out
    assert f"https://hybrid-analysis.com/sample/{t.sha256}" in out
    assert "inside `RealHiRes-1-windows-x64.zip`" in out
    assert "no automatic scan results yet" in out and "No engine reports" not in out
