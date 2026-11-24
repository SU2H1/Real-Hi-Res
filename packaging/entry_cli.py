"""PyInstaller entry point for the command-line tool (realhires-cli)."""
import multiprocessing
import sys

if __name__ == "__main__":
    multiprocessing.freeze_support()  # worker processes re-enter here
    from realhires.cli import main

    sys.exit(main())
