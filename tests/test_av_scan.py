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


def test_require_results_holds_the_release_back(tmp_path, monkeypatch):
    for k in ("VT_API_KEY", "KASPERSKY_OPENTIP_API_KEY", "HYBRID_ANALYSIS_API_KEY"):
        monkeypatch.delenv(k, raising=False)
    t = _target(tmp_path)
    notes_in, notes_out = tmp_path / "in.md", tmp_path / "out.md"
    notes_in.write_text("notes\n", encoding="utf-8")
    args = [t.path, "--tag", "v1", "--notes-in", str(notes_in), "--notes-out", str(notes_out)]
    # Without a VirusTotal key nothing is scanned, so a release must not go out.
    assert av_scan.main(args + ["--require-results"]) == 1
    assert notes_out.exists()               # the notes are still written for the log
    assert av_scan.main(args) == 0          # the daily refresh does not gate anything

    configured = {"VirusTotal": True}
    vt, kas = VirusTotal.parse(t, _report(t.sha256))
    assert av_scan.missing_results([t], {t.sha256: {"VirusTotal": vt}}, configured) == []
    pending, _ = VirusTotal.parse(t, None)
    missing = av_scan.missing_results([t], {t.sha256: {"VirusTotal": pending}}, configured)
    assert missing == ["RealHiRes.exe (VirusTotal: not analysed yet)"]
    # A detection is a finished result: it is explained in the notes, not a reason to hold back.
    det, _ = VirusTotal.parse(t, _report(t.sha256, flagged=[("Zillya", "Trojan.Stealer.Win64.2764")]))
    assert av_scan.missing_results([t], {t.sha256: {"VirusTotal": det}}, configured) == []


def test_hybrid_analysis_overview_parsing(tmp_path):
    t = _target(tmp_path)
    # The reply format Hybrid Analysis gave for the v1.8.1 launchers.
    ov = {"sha256": t.sha256, "last_file_name": "RealHiRes.exe", "threat_score": None,
          "verdict": "no specific threat", "multiscan_result": 0, "tags": ["evasive"],
          "scanners": [{"name": "CrowdStrike Falcon", "status": "no-classification", "positives": None,
                        "total": None},
                       {"name": "Metadefender", "status": "clean", "positives": 0, "total": 25}]}
    r = HybridAnalysis.parse(t, ov)
    assert r.status == "clean" and r.summary == "no specific threat, AV 0/25" and not r.detections
    bad = dict(ov, verdict="malicious", threat_score=88,
               scanners=[{"name": "Metadefender", "status": "malicious", "positives": 3, "total": 25}])
    r = HybridAnalysis.parse(t, bad)
    assert r.status == "detected" and r.detections == [("Metadefender (Hybrid Analysis)", "3/25 engines")]
    assert "threat score 88/100" in r.summary
    assert HybridAnalysis.parse(t, dict(ov, verdict="no verdict")).status == "pending"
    assert HybridAnalysis.parse(t, None).status == "pending"


def test_hybrid_analysis_submits_unknown_file_once(tmp_path):
    t = _target(tmp_path)
    calls = []

    def transport(method, url, headers=None, data=None, timeout=0):
        calls.append((method, url.split("/api/v2")[1]))
        assert headers["api-key"] == "k"
        if url.endswith(f"/overview/{t.sha256}"):
            done = any(m == "POST" for m, _ in calls) and len([c for c in calls if c[0] == "GET"]) > 2
            return (200, json.dumps({"verdict": "no specific threat"}).encode()) if done else (404, b"{}")
        if url.endswith("/submit/file"):
            assert b'name="environment_id"' in data
            return 201, json.dumps({"job_id": "j", "sha256": t.sha256}).encode()
        return 500, b""

    clock = [0.0]
    ha = HybridAnalysis("k", transport=transport, sleep=lambda s: clock.__setitem__(0, clock[0] + s),
                        clock=lambda: clock[0])
    ha.start(t)
    assert calls == [("GET", f"/overview/{t.sha256}"), ("POST", "/submit/file")]
    assert ha.check(t, deadline=600).status == "clean"
    assert sum(1 for m, _ in calls if m == "POST") == 1


def test_hybrid_analysis_does_not_wait_for_a_refused_submission(tmp_path):
    t = _target(tmp_path)
    calls = []

    def transport(method, url, headers=None, data=None, timeout=0):
        calls.append(method)
        if url.endswith("/submit/file"):  # what Hybrid Analysis answered on 2026-11-21
            return 404, json.dumps({"message": "Requested URI - Not Found"}).encode()
        return 404, json.dumps({"message": "Not Found"}).encode()

    slept = []
    ha = HybridAnalysis("k", transport=transport, sleep=slept.append, clock=lambda: 0.0)
    ha.start(t)
    r = ha.check(t, deadline=3600)
    assert r.status == "pending" and not slept, "polled a report that can never appear"
    assert r.summary == "not in Hybrid Analysis yet (uploading needs a premium API key)"


def test_hybrid_analysis_falls_back_to_the_quick_scan(tmp_path):
    t = _target(tmp_path)
    polls = []

    def transport(method, url, headers=None, data=None, timeout=0):
        path = url.split("/api/v2")[1]
        if path.startswith("/overview/"):
            return 404, b'{"message": "Not Found"}'
        if path == "/submit/file":
            return 404, b'{"message": "Requested URI - Not Found"}'
        if path == "/quick-scan/state":
            return 200, json.dumps([{"id": "lnk", "available": True}, {"id": "all", "available": True}]).encode()
        if path == "/quick-scan/file":
            assert b'name="scan_type"\r\n\r\nall' in data
            return 200, json.dumps({"id": "q1", "finished": False}).encode()
        if path == "/quick-scan/q1":
            polls.append(1)
            done = len(polls) > 1
            return 200, json.dumps({"id": "q1", "finished": done, "scanners": [
                {"name": "Metadefender", "status": "clean", "positives": 0, "total": 21}] if done else []}).encode()
        return 500, b""

    clock = [0.0]
    ha = HybridAnalysis("k", transport=transport, sleep=lambda s: clock.__setitem__(0, clock[0] + s),
                        clock=lambda: clock[0])
    ha.start(t)
    r = ha.check(t, deadline=600)
    assert r.status == "clean" and r.summary == "AV 0/21 (quick scan)"
    assert HybridAnalysis.pick_scan_type({"scanners": [{"name": "Lnk", "available": False}, {"id": "x"}]}) == "x"


def test_maintainer_review_replaces_the_warning_until_new_engines_flag(tmp_path):
    t = _target(tmp_path, "RealHiRes-1.10.0-macos-arm64.dmg", b"dmg" * 50)
    flagged = [("Microsoft", "Trojan:Script/Wacatac.C!ml")]
    reviews = {t.sha256: {"date": "2026-11-22", "engines": ["Microsoft"], "note": "same files as the clean zip."}}
    vt, kas = VirusTotal.parse(t, _report(t.sha256, flagged=flagged))
    rows = {"VirusTotal": vt, "Kaspersky": kas}
    assert "manual check" in av_scan.explain(t, rows, "")            # unreviewed: a major vendor needs a check
    text = av_scan.explain(t, rows, "", reviews)
    assert "Checked by the maintainer on 2026-11-22: same files as the clean zip." in text
    assert "manual check" not in text and "Wacatac" in text
    vt, kas = VirusTotal.parse(t, _report(t.sha256, flagged=flagged + [("ESET-NOD32", "Generik.ABC")]))
    text = av_scan.explain(t, {"VirusTotal": vt, "Kaspersky": kas}, "", reviews)
    assert "ESET-NOD32 flagged the file since" in text and "manual check" in text


def test_shipped_reviews_file_is_valid():
    reviews = av_scan.load_reviews()
    assert reviews and all(len(k) == 64 and v["engines"] and v["note"] for k, v in reviews.items())
