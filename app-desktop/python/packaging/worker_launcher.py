import sys

if __name__ == "__main__":
    if sys.argv[1:] == ["--version"]:
        from worker._version import __version__

        print(f"ai-worker {__version__}")
        raise SystemExit(0)

    from worker.__main__ import main

    raise SystemExit(main())
