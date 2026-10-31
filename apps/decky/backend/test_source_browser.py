import asyncio
import json
import types
import unittest
from threading import Event
from unittest.mock import AsyncMock, patch
import source_browser


class Context:
    def __init__(self, value): self.value = value
    async def __aenter__(self): return self.value
    async def __aexit__(self, *args): pass


class BrowserTests(unittest.IsolatedAsyncioTestCase):
    def environment(self, payloads):
        queue = asyncio.Queue()
        expressions = []
        shared = dict(title='SharedJSContext', webSocketDebuggerUrl='ws://127.0.0.1:8080/shared')
        target = dict(url='about:blank#patchhub-owned', webSocketDebuggerUrl='ws://127.0.0.1:8080/owned')
        class Socket:
            async def send_json(self, value):
                await queue.put(types.SimpleNamespace(data=json.dumps({'id':value['id'],'result':{}})))
            async def packets(self):
                while True: yield await queue.get()
            def __aiter__(self): return self.packets()
        class Session:
            async def __aenter__(self): return self
            async def __aexit__(self, *args): pass
            def ws_connect(self, url): return Context(Socket())
        async def evaluate(client, tab, expression):
            expressions.append(expression)
            if '].LoadURL(' in expression:
                for payload in payloads:
                    await queue.put(types.SimpleNamespace(type=1, data=json.dumps({
                        'method':'Runtime.bindingCalled','params':{'name':'__patchhubSourceResult','payload':json.dumps(payload)}})))
        module = types.SimpleNamespace(ClientSession=Session, WSMsgType=types.SimpleNamespace(TEXT=1, ERROR=2))
        return [patch.dict('sys.modules',{'aiohttp':module}),
                patch.object(source_browser,'uuid4',return_value=types.SimpleNamespace(hex='owned')),
                patch.object(source_browser,'_targets',new=AsyncMock(side_effect=[[shared],[shared,target]])),
                patch.object(source_browser,'_evaluate',side_effect=evaluate)], expressions

    async def run_with(self, payloads, cancel=None):
        contexts, expressions = self.environment(payloads)
        from contextlib import ExitStack
        with ExitStack() as stack:
            for context in contexts: stack.enter_context(context)
            phases=[]
            pages=await source_browser.collect_source_pages('https://www.patch.moe/?flag=1',cancel or Event(),phases.append)
        return pages, phases, expressions

    async def test_waits_for_site_javascript_and_uses_a_hidden_owned_view(self):
        pages, phases, expressions = await self.run_with([{'waiting':True},{'pages':['list']}])
        self.assertEqual(pages,['list'])
        self.assertIn('browser_verification',phases)
        self.assertEqual(phases[-1],'browser_collect')
        self.assertIn('SetVisible(false)',expressions[0])
        self.assertNotIn('window.open',expressions[0])
        self.assertNotIn('.click(',''.join(expressions))
        self.assertIn('Destroy',expressions[-1])

    async def test_required_personal_declaration_fails_and_destroys_the_view(self):
        contexts, expressions = self.environment([{'error':'source_age_verification_required'}])
        from contextlib import ExitStack
        with ExitStack() as stack:
            for context in contexts: stack.enter_context(context)
            with self.assertRaisesRegex(ValueError,'source_age_verification_required'):
                await source_browser.collect_source_pages('https://www.patch.moe/?flag=1',Event(),lambda _:None)
        self.assertIn('Destroy',expressions[-1])

    async def test_cancel_ends_waiting_and_cleans_up_without_a_result(self):
        cancel=Event();cancel.set()
        with self.assertRaisesRegex(ValueError,'task_cancelled'):
            await self.run_with([{'waiting':True}],cancel)

    def test_debug_socket_stays_local(self):
        for url in ['ws://other.example:8080/page','wss://127.0.0.1:8080/page','ws://127.0.0.1:9222/page']:
            with self.assertRaisesRegex(ValueError,'invalid_source_browser_socket'):
                source_browser._socket({'webSocketDebuggerUrl':url})

if __name__ == '__main__': unittest.main()
