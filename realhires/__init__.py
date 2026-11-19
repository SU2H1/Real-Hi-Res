"""Real-Hi-Res: detect fake high-resolution audio files.

The package decodes audio with ffmpeg and runs two independent forensic tests:

* a spectral test that looks for the brick-wall low-pass left behind when a
  44.1/48 kHz (or lossy) master is upsampled to 88.2/96/176.4/192 kHz, and
* a bit-depth test that measures how many bits of each sample are actually used,
  which exposes 16-bit audio padded with zeros into a 24-bit container.
"""

__version__ = "1.6.0"
