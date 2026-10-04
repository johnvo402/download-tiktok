from jc_worker_protocol import serve
from downloader_worker._version import __version__

def dispatch(request, emit):
    operation = request.get('operation')
    if operation == 'downloader_inspect':
        import playwright.sync_api
        import curl_cffi.requests
        emit({'type': 'capabilities', 'workerInstalled': True})
    elif operation == 'tiktok_drama_download':
        from downloader_worker.tiktok_drama.service import download_drama
        download_drama(request, emit)
    elif operation == 'tiktok_browser_login':
        from downloader_worker.tiktok_drama.browser_session import capture_browser_session
        capture_browser_session(request, emit)
    else:
        raise ValueError('Unsupported downloader operation')

def main():
    return serve('downloader', __version__, dispatch)

if __name__ == '__main__':
    raise SystemExit(main())
