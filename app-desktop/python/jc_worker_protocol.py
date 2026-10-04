"""Shared, standard-library-only JSONL transport. No models or SDKs at startup."""
from __future__ import annotations
import json
import logging
import sys
from collections.abc import Callable

PROTOCOL_VERSION = 2
MAX_REQUEST_BYTES = 8 * 1024 * 1024

def sanitize_unicode(value: object) -> object:
    if isinstance(value, str):
        return value.encode('utf-16', 'surrogatepass').decode('utf-16', 'replace')
    if isinstance(value, list):
        return [sanitize_unicode(item) for item in value]
    if isinstance(value, dict):
        return {sanitize_unicode(key): sanitize_unicode(item) for key, item in value.items()}
    return value

def serve(kind: str, version: str, handler: Callable) -> int:
    output = sys.stdout
    sys.stdout = sys.stderr
    for stream in (sys.stdin, output, sys.stderr):
        try:
            stream.reconfigure(encoding='utf-8', errors='strict' if stream is sys.stdin else 'backslashreplace')
        except (AttributeError, OSError):
            pass
    logging.basicConfig(stream=sys.stderr, level=logging.INFO)
    request_id = None
    def emit(message: dict) -> None:
        envelope = {**message, 'protocolVersion': PROTOCOL_VERSION, 'workerVersion': version,
                    'workerKind': kind, 'requestId': request_id}
        output.write(json.dumps(sanitize_unicode(envelope), ensure_ascii=False, separators=(',', ':')) + '\n')
        output.flush()
    # Bounded readline avoids allocating an unbounded malicious/malformed request.
    while True:
        raw = sys.stdin.readline(MAX_REQUEST_BYTES + 1)
        if not raw:
            break
        request_id = None
        try:
            if len(raw.encode('utf-8')) > MAX_REQUEST_BYTES:
                emit({'type': 'failed', 'code': 'REQUEST_TOO_LARGE', 'message': 'Worker request is too large.'})
                break
            request = sanitize_unicode(json.loads(raw))
            if not isinstance(request, dict):
                raise ValueError('Worker request must be a JSON object.')
            request_id = request.get('requestId')
            if request_id is not None and (not isinstance(request_id, str) or len(request_id) > 128):
                request_id = None
                raise ValueError('Invalid request ID.')
            if request.get('protocolVersion', PROTOCOL_VERSION) != PROTOCOL_VERSION:
                emit({'type': 'failed', 'code': 'PROTOCOL_MISMATCH', 'message': 'Worker protocol is not compatible.'})
                continue
            operation = request.get('operation')
            if operation in {'ping', 'health', 'version'}:
                emit({'type': 'ready', 'status': 'ready'})
            elif operation == 'shutdown':
                emit({'type': 'completed', 'status': 'stopped'})
                break
            else:
                handler(request, emit)
        except Exception as error:
            # Request payloads (cookies, URLs or tokens) must never be logged.
            logging.error('Worker request failed: %s', type(error).__name__)
            emit({'type': 'failed', 'code': 'WORKER_ERROR', 'message': str(error)})
    return 0
