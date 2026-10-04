import sys
from downloader_worker._version import __version__

if __name__ == '__main__':
    if '--version' in sys.argv:
        print(f'downloader-worker {__version__}')
    else:
        from downloader_worker.__main__ import main
        raise SystemExit(main())
