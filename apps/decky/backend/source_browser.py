"""Run the site's JavaScript in an owned, invisible Steam BrowserView."""
import asyncio
import json
from pathlib import Path
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


async def collect_source_pages(source_url, cancel, progress):
    """No age choice is submitted. A required declaration remains a failure."""
    from aiohttp import ClientSession, WSMsgType
    parsed = urlsplit(source_url)
    if parsed.scheme != 'https' or parsed.hostname != 'www.patch.moe' or parsed.port not in (None, 443):
        raise ValueError('source_browser_origin_changed')
    script_path = Path(__file__).with_name('shiravune-browser.js')
    if not script_path.is_file():
        script_path = Path(__file__).parents[2] / 'windows-flutter/assets/shiravune-browser.js'
    script = script_path.read_text(encoding='utf-8')
    token = uuid4().hex
    key = json.dumps(token)

    async def run():
        async with ClientSession() as client:
            targets = await _targets(client)
            shared = next((target for target in targets if target.get('title') == 'SharedJSContext'), None)
            if shared is None:
                raise ValueError('source_browser_unavailable')
            try:
                progress('browser_verification')
                await _evaluate(client, shared, '''(async () => {
                  const view = SteamClient.BrowserView.Create();
                  const views = window.__patchhubSourceViews ??= {};
                  views[TOKEN] = view;
                  const ready = new Promise((resolve, reject) => {
                    view.on('finished-request', () => resolve(true));
                    view.on('load-error', (_code, _url, error) => reject(new Error(error)));
                  });
                  const monitor = await SteamClient.Window.GetDefaultMonitorDimensions();
                  view.SetBounds(0, 0, monitor.nUsableWidth, monitor.nUsableHeight);
                  view.SetVisible(false);
                  view.LoadURL('about:blank#patchhub-' + TOKEN);
                  return await ready;
                })()'''.replace('TOKEN', key))
                targets = await _targets(client)
                target = next((target for target in targets if target.get('url') == 'about:blank#patchhub-' + token), None)
                if target is None:
                    raise ValueError('source_browser_not_found')
                async with client.ws_connect(_socket(target)) as ws:
                    await _command(ws, 1, 'Page.enable', {})
                    await _command(ws, 2, 'Runtime.addBinding', {'name': '__patchhubSourceResult'})
                    await _command(ws, 3, 'Page.addScriptToEvaluateOnNewDocument', {'source':
                        "if (window === window.top) addEventListener('load', () => { (" + script +
                        ").then(value => window.__patchhubSourceResult(JSON.stringify(value))); });"})
                    await _evaluate(client, shared,
                        'window.__patchhubSourceViews[' + key + '].LoadURL(' + json.dumps(source_url) + ')')
                    async for message in ws:
                        if message.type == WSMsgType.ERROR:
                            raise ValueError('source_browser_connection_failed')
                        if message.type != WSMsgType.TEXT:
                            continue
                        packet = json.loads(message.data)
                        if packet.get('method') != 'Runtime.bindingCalled':
                            continue
                        params = packet['params']
                        if params.get('name') != '__patchhubSourceResult':
                            continue
                        data = json.loads(params['payload'])
                        if data.get('waiting'):
                            progress('browser_verification')
                        elif data.get('error'):
                            raise ValueError(data['error'])
                        else:
                            progress('browser_collect')
                            return data['pages']
                    raise ValueError('source_browser_closed')
            finally:
                await _evaluate(client, shared, '''(() => {
                  const views = window.__patchhubSourceViews;
                  if (views?.[TOKEN]) { SteamClient.BrowserView.Destroy(views[TOKEN]); delete views[TOKEN]; }
                })()'''.replace('TOKEN', key))

    async def cancelled():
        while not cancel.is_set():
            await asyncio.sleep(0.1)
        raise ValueError('task_cancelled')

    work = asyncio.create_task(run())
    stop = asyncio.create_task(cancelled())
    try:
        done, _ = await asyncio.wait((work, stop), return_when=asyncio.FIRST_COMPLETED)
        if stop in done:
            await stop
        return await work
    finally:
        work.cancel()
        stop.cancel()
        await asyncio.gather(work, stop, return_exceptions=True)
