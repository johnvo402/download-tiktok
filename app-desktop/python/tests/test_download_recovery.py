"""Offline fixtures; no browser, cookies, model or live TikTok request."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from downloader_worker.tiktok_drama.direct_media import (
    VideoCandidate, _probe_playable, download_video_candidate,
)


def mp4_fixture():
    def box(kind, data):
        return (len(data) + 8).to_bytes(4, 'big') + kind + data
    return box(b'ftyp', b'isom0000') + box(b'moov', b'metadata') + box(b'mdat', b'x' * 160_000)


class Response:
    url = 'https://v16.tiktokcdn.com/video.mp4'
    def __init__(self, data, status=200, start=0, total=None, fail=False, mime='video/mp4'):
        self.data, self.fail = data, fail
        self.status_code = status
        self.headers = {'content-type': mime, 'etag': '"fixture-1"', 'content-length': str(len(data))}
        if status == 206:
            self.headers['content-range'] = f'bytes {start}-{total-1}/{total}'
    def iter_content(self, chunk_size):
        if self.fail:
            yield self.data[:100_000]
            raise OSError('offline fixture connection reset')
        yield self.data
    def close(self): pass


class Session:
    def __init__(self, responses):
        self.responses, self.requests = iter(responses), []
        self.cookies = SimpleNamespace(set=lambda *a, **k: None)
    def get(self, url, **kwargs):
        self.requests.append(kwargs['headers'])
        return next(self.responses)
    def close(self): pass


class DownloadRecoveryTests(unittest.TestCase):
    def run_download(self, responses):
        fixture = Session(responses)
        self.enterContext(patch.dict(sys.modules, {'curl_cffi': SimpleNamespace(requests=SimpleNamespace(Session=lambda **kwargs: fixture))}))
        self.enterContext(patch('downloader_worker.tiktok_drama.direct_media.time.sleep'))
        self.enterContext(patch.dict(os.environ, {'JC_FFPROBE_PATH': ''}))
        temp = self.enterContext(tempfile.TemporaryDirectory())
        return fixture, Path(temp)

    def test_network_failure_continues_range_and_publishes_only_complete_file(self):
        data = mp4_fixture()
        session, folder = self.run_download([Response(data, fail=True), Response(data[100_000:], 206, 100_000, len(data))])
        output = download_video_candidate(VideoCandidate(Response.url), [], 1, folder, lambda *args: None)
        self.assertEqual(output.read_bytes(), data)
        self.assertEqual(session.requests[1]['Range'], 'bytes=100000-')
        self.assertEqual(session.requests[1]['If-Range'], '"fixture-1"')
        self.assertEqual([p.name for p in folder.iterdir()], ['EP0001.mp4'])

    def test_server_ignores_range_restarts_without_appending_duplicate_bytes(self):
        data = mp4_fixture()
        session, folder = self.run_download([Response(data, fail=True), Response(data)])
        output = download_video_candidate(VideoCandidate(Response.url), [], 1, folder, lambda *args: None)
        self.assertEqual(output.read_bytes(), data)

    def test_bad_range_and_non_video_are_never_published(self):
        for response in [Response(mp4_fixture(), 206, 20, 160_000), Response(b'<html>error</html>', mime='text/html')]:
            with self.subTest(response=response):
                _, folder = self.run_download([response])
                with self.assertRaises(RuntimeError):
                    download_video_candidate(VideoCandidate(Response.url), [], 1, folder, lambda *args: None)
                self.assertFalse(list(folder.iterdir()))

    def test_unsafe_redirect_is_rejected_before_connecting_to_target(self):
        response = Response(b'', status=302)
        response.headers['location'] = 'http://127.0.0.1/private'
        session, folder = self.run_download([response])
        with self.assertRaises(RuntimeError):
            download_video_candidate(VideoCandidate(Response.url), [], 1, folder, lambda *args: None)
        self.assertEqual(len(session.requests), 1)
        self.assertFalse(list(folder.iterdir()))

    def test_ffprobe_checks_duration_stream_and_timeout(self):
        with patch.dict(os.environ, {'JC_FFPROBE_PATH': 'bundled-ffprobe'}):
            for body, valid in [({'format': {'duration': '2'}, 'streams': [{'codec_type': 'video', 'width': 160, 'height': 120}]}, True),
                                ({'format': {'duration': 'nan'}, 'streams': []}, False),
                                ({'format': {'duration': '2'}, 'streams': [{'codec_type': 'audio'}]}, False)]:
                with patch('subprocess.run', return_value=SimpleNamespace(returncode=0, stdout=json.dumps(body).encode())) as run:
                    self.assertEqual(_probe_playable(Path('tiếng Việt.mp4')), valid)
                    self.assertEqual(run.call_args.kwargs['timeout'], 30)
            with patch('subprocess.run', side_effect=subprocess.TimeoutExpired('ffprobe', 30)):
                self.assertFalse(_probe_playable(Path('fixture.mp4')))
