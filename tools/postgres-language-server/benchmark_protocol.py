#!/usr/bin/env python3
"""Offline completion-process benchmark; no database or native app interaction."""
import argparse
import json
import math
import os
from pathlib import Path
import selectors
import subprocess
import tempfile
import time


class Session:
    def __init__(self, binary, catalog):
        self.stderr = tempfile.TemporaryFile()
        self.process = subprocess.Popen(
            [str(binary), "--completion-catalog", str(catalog)],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.stderr,
        )
        self.selector = selectors.DefaultSelector()
        self.selector.register(self.process.stdout, selectors.EVENT_READ)
        self.buffer = bytearray()
        self.request_id = 0

    def send(self, method, params=None, request=False):
        message = {"jsonrpc": "2.0", "method": method, "params": params or {}}
        if request:
            self.request_id += 1
            message["id"] = self.request_id
        body = json.dumps(message).encode()
        self.process.stdin.write(f"Content-Length: {len(body)}\r\n\r\n".encode() + body)
        self.process.stdin.flush()
        if request:
            response = self.receive()
            assert response.get("id") == self.request_id, response
            assert "error" not in response, response
            return response["result"]

    def receive(self):
        deadline = time.monotonic() + 15
        while True:
            boundary = self.buffer.find(b"\r\n\r\n")
            if boundary >= 0:
                headers = dict(line.split(b":", 1) for line in bytes(self.buffer[:boundary]).split(b"\r\n"))
                length = int(headers[b"Content-Length"])
                end = boundary + 4 + length
                if len(self.buffer) >= end:
                    body = bytes(self.buffer[boundary + 4:end])
                    del self.buffer[:end]
                    return json.loads(body)
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not self.selector.select(remaining):
                raise TimeoutError("Completion helper did not answer within 15 seconds")
            chunk = os.read(self.process.stdout.fileno(), 65536)
            if not chunk:
                raise RuntimeError("Completion helper closed stdout")
            self.buffer.extend(chunk)

    def rss_kib(self):
        return int(subprocess.check_output(
            ["ps", "-o", "rss=", "-p", str(self.process.pid)], text=True).strip())

    def close(self):
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
        self.selector.close()
        self.process.stdin.close()
        self.process.stdout.close()
        self.stderr.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/pg-language-server/release/rustable-postgres-language-server"))
    args = parser.parse_args()
    catalog = {"tables": [{"schema": "public", "name": f"table_{i}", "columns": [
        {"name": f"column_{j}"} for j in range(20)
    ]} for i in range(5000)]}
    text = "SELECT a.column_1 FROM public.table_4999 a JOIN public.table_2500 b ON a.column_0 = b.column_0 JOIN public.table_42 c ON c.column_0 = a.column_0"
    uri = "file:///rustable-benchmark.sql"
    params = {"textDocument": {"uri": uri}, "position": {"line": 0, "character": text.index("column_1") + 8}}
    with tempfile.TemporaryDirectory(prefix="rustable-completion-bench-") as directory:
        path = Path(directory) / "catalog.json"
        path.write_text(json.dumps(catalog))
        started = time.perf_counter()
        session = Session(args.binary.resolve(), path)
        try:
            session.send("initialize", request=True)
            startup_ms = (time.perf_counter() - started) * 1000
            loaded_rss = session.rss_kib()
            session.send("textDocument/didOpen", {"textDocument": {"uri": uri, "text": text}})
            timings = []
            for i in range(505):
                start = time.perf_counter()
                result = session.send("textDocument/completion", params, request=True)
                elapsed = (time.perf_counter() - start) * 1000
                assert any(item["label"] == "column_19" for item in result["items"])
                if i >= 5:
                    timings.append(elapsed)
            session.send("textDocument/didClose", {"textDocument": {"uri": uri}})
            # Fill all eight document slots near the 1 MiB/document limit, then
            # close them repeatedly. A completion response acts as a FIFO barrier.
            retained_rss, closed_rss = [], []
            document = text + "\n/*" + "x" * 1_000_000 + "*/"
            for cycle in range(10):
                for index in range(8):
                    session.send("textDocument/didOpen", {"textDocument": {
                        "uri": f"file:///cycle-{index}.sql", "text": document,
                    }})
                for index in range(8):
                    retained_params = {**params, "textDocument": {"uri": f"file:///cycle-{index}.sql"}}
                    result = session.send("textDocument/completion", retained_params, request=True)
                    assert any(item["label"] == "column_19" for item in result["items"])
                session.send("textDocument/didOpen", {"textDocument": {"uri": uri, "text": text}})
                result = session.send("textDocument/completion", params, request=True)
                assert result["items"] == [], "Ninth document must not exceed the retention bound"
                retained_rss.append(session.rss_kib())
                for index in range(8):
                    session.send("textDocument/didClose", {"textDocument": {"uri": f"file:///cycle-{index}.sql"}})
                    closed_params = {**params, "textDocument": {"uri": f"file:///cycle-{index}.sql"}}
                    result = session.send("textDocument/completion", closed_params, request=True)
                    assert result["items"] == [], "Closed document still returned completion"
                closed_rss.append(session.rss_kib())
            timings.sort()
            report = {
                "tables": 5000, "columns": 100000, "catalog_bytes": path.stat().st_size,
                "startup_to_initialize_ms": round(startup_ms, 3),
                "requests": len(timings),
                "protocol_latency_ms": {f"p{p}": round(timings[math.ceil(len(timings) * p / 100) - 1], 3) for p in (50, 95, 99)},
                "rss_after_catalog_kib": loaded_rss,
                "rss_eight_documents_kib": retained_rss,
                "rss_after_close_kib": closed_rss,
            }
            session.send("shutdown", request=True)
            session.send("exit")
            assert session.process.wait(timeout=5) == 0
            print(json.dumps(report, indent=2))
        finally:
            session.close()


if __name__ == "__main__":
    main()
