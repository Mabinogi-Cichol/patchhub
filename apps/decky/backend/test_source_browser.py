import asyncio
import json
import unittest
from threading import Event
from types import SimpleNamespace
from unittest.mock import AsyncMock, patch

from source_browser import SourceBrowserSession


class Socket:
    def __init__(self, packets):
        self.packets = iter(packets)
        self.sent = []

    async def __aenter__(self):
        return self

    async def __aexit__(self, *_):
        pass

    async def send_json(self, value):
        self.sent.append(value)

    async def __aiter__(self):
        for packet in self.packets:
            yield SimpleNamespace(data=json.dumps(packet))


class BrowserNavigationTest(unittest.IsolatedAsyncioTestCase):
    def session(self, packets):
        socket = Socket(packets)
        session = SourceBrowserSession(None)
        session.client = SimpleNamespace(ws_connect=lambda _: socket)
        session.target = {'webSocketDebuggerUrl': 'ws://localhost:8080/devtools/page/fixture'}
        return session, socket

    async def test_document_status_excludes_iframe_response(self):
        url = 'https://example.com/'
        session, _ = self.session([
            {'method': 'Network.responseReceived', 'params': {
                'type': 'Document', 'frameId': 'main',
                'response': {'url': url, 'status': 403}}},
            {'method': 'Network.responseReceived', 'params': {
                'type': 'Document', 'frameId': 'child',
                'response': {'url': url, 'status': 200}}},
            {'method': 'Page.loadEventFired'},
        ])
        with patch('source_browser._command', new=AsyncMock(return_value={
            'frameTree': {'frame': {'id': 'main'}},
        })), patch('source_browser._evaluate', new=AsyncMock(return_value={
            'response_url': url, 'text': '<html>verification</html>',
        })):
            self.assertEqual((await session._get(url, ['example.com']))['status'], 403)

    async def test_redirect_outside_source_hosts_is_rejected_before_resume(self):
        session, socket = self.session([{'method': 'Fetch.requestPaused', 'params': {
            'requestId': 'redirect', 'request': {'url': 'https://outside.example/'},
        }}])
        with patch('source_browser._command', new=AsyncMock(return_value={
            'frameTree': {'frame': {'id': 'main'}},
        })):
            with self.assertRaisesRegex(ValueError, 'source_browser_origin_changed'):
                await session._get('https://example.com/', ['example.com'])
        self.assertEqual(socket.sent[-1]['method'], 'Fetch.failRequest')

    async def test_cancellation_waits_for_browser_request_cleanup(self):
        cancel = Event()
        session = SourceBrowserSession(cancel)
        entered = asyncio.Event()
        released = asyncio.Event()

        async def request(_):
            entered.set()
            try:
                await asyncio.Future()
            finally:
                released.set()

        with patch.object(session, '_request', new=request):
            task = asyncio.create_task(session.request({}))
            await asyncio.wait_for(entered.wait(), 5)
            cancel.set()
            with self.assertRaisesRegex(ValueError, 'task_cancelled'):
                await task
        self.assertTrue(released.is_set())
