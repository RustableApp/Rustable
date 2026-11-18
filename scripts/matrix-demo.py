#!/usr/bin/env python3
"""Launch the Matrix experiment and an optional scripted tour through real MCP."""
import argparse
import json
import pathlib
import os
import shutil
import subprocess
import tempfile
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--fixture-tour', action='store_true', help='QA-only tour against RUSTABLE_MATRIX_FIXTURE')
parser.add_argument('--edit-tour', action='store_true', help='QA-only progressive edit of an existing SQL tab')
parser.add_argument('--connections', action='store_true', help='Open the real connection manager; no scripted queries')
parser.add_argument('--binary', default='target/debug/rustable')
parser.add_argument('--exit-after-tour', action='store_true', help='Exit after verification (for QA)')
parser.add_argument('--no-tour', action='store_true', help='Launch the workspace without scripted actions')
args = parser.parse_args()
if args.fixture_tour and not (args.connections and os.environ.get('RUSTABLE_MATRIX_FIXTURE') and os.environ.get('RUSTABLE_MATRIX_CAPTURE')):
    parser.error('--fixture-tour requires --connections and the explicit QA fixture/capture environment')
if args.edit_tour and (args.connections or not os.environ.get('RUSTABLE_MATRIX_EDIT_QA')):
    parser.error('--edit-tour requires the disposable RUSTABLE_MATRIX_EDIT_QA fixture')
binary = str(pathlib.Path(args.binary).resolve())
directory = tempfile.mkdtemp(prefix='rustable-matrix-')
socket = str(pathlib.Path(directory) / 'workspace.sock')
app = subprocess.Popen([binary, 'matrix' if args.connections else 'matrix-demo', '--socket', socket])
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
    if not args.no_tour and (not args.connections or args.fixture_tour):
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

        workspace_id = None
        def call(name, arguments):
            if workspace_id is not None and name != 'list_workspaces':
                arguments = dict(arguments, workspace_id=workspace_id)
            result = rpc('tools/call', {'name': name, 'arguments': arguments})
            if result.get('isError'):
                raise RuntimeError(result['content'][0]['text'])
            return json.loads(result['content'][0]['text'])

        rpc('initialize', {'protocolVersion': '2024-11-05', 'capabilities': {},
                           'clientInfo': {'name': 'matrix-scripted-tour', 'version': '1'}})
        for _ in range(100):
            workspaces = call('list_workspaces', {})['workspaces']
            if workspaces:
                workspace_id = workspaces[0]['workspace_id']
                break
            time.sleep(0.1)
        if workspace_id is None:
            raise TimeoutError('No enabled workspace appeared; enable AI control in the intended window')
        for _ in range(100):
            if not call('workspace', {})['busy']:
                break
            time.sleep(0.1)
        if args.edit_tour:
            for _ in range(100):
                workspace = call('workspace', {})
                tab = workspace.get('active_tab_id')
                if tab is not None:
                    current = call('inspect_query', {'tab_id': tab})
                    if current['sql'] == 'SELECT plan, COUNT(*) AS customers\nFROM customers\nGROUP BY plan;\n':
                        break
                time.sleep(0.1)
            else:
                raise RuntimeError('Expected untouched QA human-tab fixture')
            assert not next(t for t in workspace['tabs'] if t['id'] == tab)['ai_owned']
            edit = call('begin_query_edit', {'tab_id': tab, 'expected_sql': current['sql'],
                        'start': 0, 'end': len(current['sql'])})['edit_id']
            chunks = ['-- Active customers by plan\n', 'SELECT plan, COUNT(*) AS customers\nFROM customers\n',
                      "WHERE status = 'Active'\n", 'GROUP BY plan\nORDER BY customers DESC;\n']
            for sequence, chunk in enumerate(chunks):
                call('append_query_edit', {'edit_id': edit, 'sequence': sequence, 'text': chunk})
                time.sleep(0.6)
            call('finish_query_edit', {'edit_id': edit})
            current = call('inspect_query', {'tab_id': tab})
            assert current['sql'] == ''.join(chunks)
            assert current['edit']['finished'] and not current['running']
            print('Edited existing human SQL tab in four visible chunks; SQL was not executed.', flush=True)
            time.sleep(1.5)
        for title, sql in ([] if args.edit_tour else [
            ('Customers by plan', '-- How is the customer base distributed?\nSELECT plan, COUNT(*) AS customers\nFROM customers\nGROUP BY plan\nORDER BY customers DESC;'),
            ('Revenue by product', '-- Which products account for paid orders?\nSELECT p.name, COUNT(*) AS orders, ROUND(SUM(o.total), 2) AS revenue\nFROM orders o\nJOIN products p ON p.id = o.product_id\nWHERE o.status = \'Paid\'\nGROUP BY p.id, p.name\nORDER BY revenue DESC;'),
            ('Customers to explore', '-- Continue here: adjust the filter or inspect a customer.\nSELECT id, name, plan, country, lifetime_value\nFROM customers\nWHERE status = \'Active\'\nORDER BY lifetime_value DESC\nLIMIT 20;'),
        ]):
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
        print('Tour complete. Continue editing; the AI toolbar menu has access and undo controls. Close the window to finish.', flush=True)
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
