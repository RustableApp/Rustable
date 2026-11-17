#!/usr/bin/env python3
"""Launch the Matrix experiment and an optional scripted tour through real MCP."""
import argparse
import json
import pathlib
import shutil
import subprocess
import tempfile
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary', default='target/debug/rustable')
parser.add_argument('--exit-after-tour', action='store_true', help='Exit after verification (for QA)')
parser.add_argument('--no-tour', action='store_true', help='Launch the workspace without scripted actions')
args = parser.parse_args()
binary = str(pathlib.Path(args.binary).resolve())
directory = tempfile.mkdtemp(prefix='rustable-matrix-')
socket = str(pathlib.Path(directory) / 'workspace.sock')
app = subprocess.Popen([binary, 'matrix', '--socket', socket])
client = None
try:
    for _ in range(200):
        if app.poll() is not None:
            raise RuntimeError('Matrix window exited before connecting')
        if pathlib.Path(socket).exists():
            break
        time.sleep(0.1)
    else:
        raise TimeoutError('Matrix window did not start')
    print('MCP client configuration for this running window:', flush=True)
    print(json.dumps({'mcpServers': {'rustable-matrix': {'command': binary,
          'args': ['mcp', '--socket', socket]}}}, indent=2), flush=True)
    if not args.no_tour:
        client = subprocess.Popen([binary, 'mcp', '--socket', socket], stdin=subprocess.PIPE,
                                  stdout=subprocess.PIPE, text=True, bufsize=1)
        sequence = 0

        def rpc(method, params):
            global sequence
            sequence += 1
            client.stdin.write(json.dumps({'jsonrpc': '2.0', 'id': sequence,
                                          'method': method, 'params': params}) + '\n')
            client.stdin.flush()
            response = json.loads(client.stdout.readline())
            if 'error' in response:
                raise RuntimeError(response['error'])
            return response['result']

        def call(name, arguments):
            result = rpc('tools/call', {'name': name, 'arguments': arguments})
            if result.get('isError'):
                raise RuntimeError(result['content'][0]['text'])
            return json.loads(result['content'][0]['text'])

        rpc('initialize', {'protocolVersion': '2024-11-05', 'capabilities': {},
                           'clientInfo': {'name': 'matrix-scripted-tour', 'version': '1'}})
        for _ in range(100):
            if not call('workspace', {})['busy']:
                break
            time.sleep(0.1)
        for title, sql in [
            ('Customers by plan', '-- How is the customer base distributed?\nSELECT plan, COUNT(*) AS customers\nFROM customers\nGROUP BY plan\nORDER BY customers DESC;'),
            ('Revenue by product', '-- Which products account for paid orders?\nSELECT p.name, COUNT(*) AS orders, ROUND(SUM(o.total), 2) AS revenue\nFROM orders o\nJOIN products p ON p.id = o.product_id\nWHERE o.status = \'Paid\'\nGROUP BY p.id, p.name\nORDER BY revenue DESC;'),
            ('Customers to explore', '-- Continue here: adjust the filter or inspect a customer.\nSELECT id, name, plan, country, lifetime_value\nFROM customers\nWHERE status = \'Active\'\nORDER BY lifetime_value DESC\nLIMIT 20;'),
        ]:
            draft = f'-- Working on: {title}\n'
            tab = call('open_query', {'title': title, 'sql': draft})['tab_id']
            time.sleep(0.7)
            call('replace_query', {'tab_id': tab, 'expected_sql': draft, 'sql': sql})
            time.sleep(0.8)
            result = call('run_query', {'tab_id': tab, 'expected_sql': sql})
            if not result['started']:
                raise RuntimeError(f'Query did not start: {result}')
            for _ in range(100):
                result = call('inspect_query', {'tab_id': tab})
                if not result['running']:
                    break
                time.sleep(0.1)
            else:
                call('cancel_query', {})
                raise TimeoutError('Tour query exceeded 10 seconds')
            if result['error']:
                raise RuntimeError(result['error'])
            print(f"{title}: {result['total_rows']} rows", flush=True)
            time.sleep(1.5)
        client.stdin.close()
        client.wait(timeout=5)
        client = None
        print('Tour complete. Click Take over and continue in any query tab. Close the window to finish.', flush=True)
    if args.exit_after_tour:
        app.terminate()
    app.wait()
finally:
    if client is not None:
        client.terminate()
    if app.poll() is None:
        app.terminate()
        app.wait(timeout=5)
    shutil.rmtree(directory)
