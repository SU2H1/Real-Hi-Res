"""Background listing of the audio files in an added folder (used by the GUI)."""

from __future__ import annotations

import os
import queue
import threading
import time
from typing import List

from .cli import iter_audio_files


def list_source(source: str, cfg, out: queue.Queue, cancel: threading.Event) -> None:
    """Find the audio files of one added folder/file (background thread).

    Files are posted in batches while the folder is walked ("listing", source,
    [paths]), so big libraries appear progressively, then ("listed", source,
    None) marks the end."""
    batch: List[str] = []
    last = time.time()
    try:
        for f in iter_audio_files(source, cfg.recursive, cfg.extension_set()):
            if cancel.is_set():
                return
            batch.append(os.path.abspath(f))
            if len(batch) >= 500 or (time.time() - last > 0.25 and batch):
                out.put(("listing", source, batch))
                batch, last = [], time.time()
    except OSError:
        pass
    if batch:
        out.put(("listing", source, batch))
    out.put(("listed", source, None))
