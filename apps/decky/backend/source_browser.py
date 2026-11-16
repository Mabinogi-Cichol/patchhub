"""Run the site's JavaScript in an owned, invisible Steam BrowserView."""
import asyncio
import json
from urllib.parse import urlsplit
from uuid import uuid4


async def _targets(client):
    async with client.get('http://127.0.0.1:8080/json/list') as response:
        response.raise_for_status()
        return await response.json()


def _socket(target):
    address = target['webSocketDebuggerUrl']
    url = urlsplit(address)
    if url.scheme != 'ws' or url.hostname not in ('localhost', '127.0.0.1') or url.port != 8080:
        raise ValueError('invalid_source_browser_socket')
    return address


async def _command(ws, command_id, method, params):
    await ws.send_json({'id': command_id, 'method': method, 'params': params})
    async for message in ws:
        packet = json.loads(message.data)
        if packet.get('id') != command_id:
            continue
        if 'error' in packet or 'exceptionDetails' in packet.get('result', {}):
            raise ValueError('source_browser_evaluation_failed')
        return packet.get('result', {})
    raise ValueError('source_browser_closed')


async def _evaluate(client, target, expression):
    async with client.ws_connect(_socket(target)) as ws:
        result = await _command(ws, 1, 'Runtime.evaluate', {
            'expression': expression, 'awaitPromise': True, 'returnByValue': True})
        return result.get('result', {}).get('value')


def _source_url(value, hosts):
    url = urlsplit(value)
    if (url.scheme != 'https' or url.hostname not in hosts
            or url.port not in (443, None) or url.username is not None):
        raise ValueError('source_browser_origin_changed')


class SourceBrowserSession:
    """One invisible BrowserView owned by a source update."""
    def __init__(self, cancel):
        self.cancel = cancel
        self.client = self.shared = self.target = None
        self.key = json.dumps(uuid4().hex)

    async def _open(self):
        from aiohttp import ClientSession
        self.client = ClientSession()
        targets = await _targets(self.client)
        self.shared = next((target for target in targets
                            if target.get('title') == 'SharedJSContext'), None)
        if self.shared is None:
            raise ValueError('source_browser_unavailable')
        await _evaluate(self.client, self.shared, '''(async () => {
          const view = SteamClient.BrowserView.Create();
          const views = window.__patchhubSourceViews ??= {};
          views[TOKEN] = view;
          const ready = new Promise((resolve, reject) => {
            view.on('finished-request', resolve);
            view.on('load-error', (_code, _url, error) => reject(new Error(error)));
          });
          const monitor = await SteamClient.Window.GetDefaultMonitorDimensions();
          view.SetBounds(0, 0, monitor.nUsableWidth, monitor.nUsableHeight);
          view.SetVisible(false);
          view.LoadURL('about:blank#patchhub-' + TOKEN);
          await ready;
        })()'''.replace('TOKEN', self.key))
        target_url = 'about:blank#patchhub-' + json.loads(self.key)
        self.target = next((target for target in await _targets(self.client)
                            if target.get('url') == target_url), None)
        if self.target is None:
            raise ValueError('source_browser_not_found')

    async def _get(self, url, hosts):
        async with self.client.ws_connect(_socket(self.target)) as ws:
            await _command(ws, 1, 'Page.enable', {})
            await _command(ws, 2, 'Fetch.enable', {
                'patterns': [{'resourceType': 'Document', 'requestStage': 'Request'}],
            })
            tree = await _command(ws, 3, 'Page.getFrameTree', {})
            main_frame = tree['frameTree']['frame']['id']
            await _command(ws, 4, 'Network.enable', {})
            await ws.send_json({'id': 5, 'method': 'Page.navigate', 'params': {'url': url}})
            sequence = 5
            document = None
            async for message in ws:
                packet = json.loads(message.data)
                if packet.get('id') == 5 and ('error' in packet
                        or packet.get('result', {}).get('errorText')):
                    raise ValueError('source_browser_load_failed')
                if packet.get('method') == 'Fetch.requestPaused':
                    paused = packet['params']
                    sequence += 1
                    try:
                        _source_url(paused['request']['url'], hosts)
                    except ValueError:
                        await ws.send_json({'id': sequence, 'method': 'Fetch.failRequest',
                            'params': {'requestId': paused['requestId'], 'errorReason': 'BlockedByClient'}})
                        raise
                    await ws.send_json({'id': sequence, 'method': 'Fetch.continueRequest',
                                        'params': {'requestId': paused['requestId']}})
                if packet.get('method') == 'Network.responseReceived':
                    event = packet['params']
                    if event.get('type') == 'Document' and event.get('frameId') == main_frame:
                        document = event['response']
                if packet.get('method') == 'Page.loadEventFired' and document is not None:
                    break
            else:
                raise ValueError('source_browser_closed')
        snapshot = await self._snapshot()
        _source_url(document['url'], hosts)
        return dict(snapshot, status=document['status'])

    async def _snapshot(self):
        return await _evaluate(self.client, self.target, '''(() => {
          const snapshot = document.documentElement.cloneNode(true);
          const live = document.querySelectorAll('input');
          snapshot.querySelectorAll('input').forEach((input, index) => {
            input.setAttribute('value', live[index].value);
          });
          const navigation = performance.getEntriesByType('navigation')[0];
          return {response_url: location.href, status: navigation?.responseStatus, text: snapshot.outerHTML};
        })()''')

    async def _request(self, request):
        url, method, hosts = (request[key] for key in ('url', 'method', 'page_hosts'))
        _source_url(url, hosts)
        if self.target is None:
            await self._open()
        if method == 'GET':
            result = await self._get(url, hosts)
        elif method == 'SNAPSHOT':
            result = await self._snapshot()
        elif method == 'POST':
            result = await _evaluate(self.client, self.target, '''(async request => {
              const destination = new URL(request.url);
              if (destination.origin !== location.origin) throw Error('origin');
              const response = await fetch(destination, {
                method: 'POST', body: request.body, redirect: 'error',
                headers: {'Content-Type': request.content_type},
                credentials: 'same-origin', mode: 'same-origin',
              });
              return {response_url: response.url, status: response.status, text: await response.text()};
            })(REQUEST)'''.replace('REQUEST', json.dumps(request)))
        else:
            raise ValueError('invalid_browser_request')
        _source_url(result['response_url'], hosts)
        if type(result.get('status')) is not int or not 100 <= result['status'] <= 599:
            raise ValueError('source_browser_load_failed')
        return dict(result, url=url, method=method, body=request.get('body'))

    async def request(self, request):
        work = asyncio.create_task(self._request(request))
        async def cancelled():
            while not self.cancel.is_set():
                await asyncio.sleep(0.1)
        stop = asyncio.create_task(cancelled())
        try:
            done, _ = await asyncio.wait((work, stop), return_when=asyncio.FIRST_COMPLETED)
            if stop in done:
                raise ValueError('task_cancelled')
            return await work
        finally:
            work.cancel()
            stop.cancel()
            await asyncio.gather(work, stop, return_exceptions=True)

    async def close(self):
        if self.client is None:
            return
        async with self.client:
            if self.shared is not None:
                await _evaluate(self.client, self.shared, '''(() => {
                  const views = window.__patchhubSourceViews;
                  if (views?.[TOKEN]) { SteamClient.BrowserView.Destroy(views[TOKEN]); delete views[TOKEN]; }
                })()'''.replace('TOKEN', self.key))
