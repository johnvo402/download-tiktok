import sys

if __name__ == "__main__":
    if sys.argv[1:] == ["--version"]:
        from worker._version import __version__

        print(f"ai-worker {__version__}")
        raise SystemExit(0)

    # yt-dlp runs as a hidden child process from the desktop worker so its CLI
    # behavior matches the standalone downloader, including packaged releases.
    if sys.argv[1:2] == ["--yt-dlp-cli"]:
        import yt_dlp

        raise SystemExit(yt_dlp.main(sys.argv[2:]))

    from worker.__main__ import main

    raise SystemExit(main())
