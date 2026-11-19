"""PyInstaller entry point for the windowed GUI (RealHiRes / RealHiRes.exe / RealHiRes.app)."""
import multiprocessing
import sys

if __name__ == "__main__":
    multiprocessing.freeze_support()  # worker processes re-enter here
    from realhires.gui import main

    sys.exit(main())
