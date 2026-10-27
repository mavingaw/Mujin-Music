"""Mujin Music's AI enhancer worker.

Restores lossy music (MP3, AAC and the like) with a generative model on the
your own GPU. The model invents plausible high frequencies the encoder
threw away; it never recovers the original master, so Mujin Music labels
everything it makes "AI-enhanced", never "lossless".
"""

__version__ = "0.1.0"
