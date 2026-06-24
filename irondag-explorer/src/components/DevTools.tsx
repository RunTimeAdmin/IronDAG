import { useState } from 'react';
import { WalletState, sendIDAG, deployContract } from '../hooks/useWallet';
import { CHAIN_ID, CHAIN_ID_HEX, TICKER, NETWORK_NAME, RPC_PUBLIC, EXPLORER_URL, DECIMALS } from '../lib/config';
import { rpcCall } from '../lib/rpc';
import { fmtWei } from '../lib/format';

type Tab = 'network' | 'send' | 'deploy' | 'interact';

interface Props { wallet: WalletState }

export function DevTools({ wallet }: Props) {
  const [tab, setTab] = useState<Tab>('network');

  const tabs: { id: Tab; label: string; icon: string }[] = [
    { id: 'network', label: 'Network Setup', icon: 'fa-network-wired' },
    { id: 'send', label: `Send ${TICKER}`, icon: 'fa-paper-plane' },
    { id: 'deploy', label: 'Deploy Contract', icon: 'fa-rocket' },
    { id: 'interact', label: 'Interact', icon: 'fa-code' },
  ];

  return (
    <section id="dev-tools" className="devtools-section">
      <div className="container">
        <h2><i className="fas fa-tools" /> Developer Tools</h2>
        <div className="devtools-card">
          <div className="devtools-tabs">
            {tabs.map(t => (
              <button key={t.id} className={`devtools-tab${tab === t.id ? ' active' : ''}`} onClick={() => setTab(t.id)}>
                <i className={`fas ${t.icon}`} /> {t.label}
              </button>
            ))}
          </div>
          <div className="devtools-content">
            {tab === 'network' && <NetworkTab wallet={wallet} />}
            {tab === 'send' && <SendTab wallet={wallet} />}
            {tab === 'deploy' && <DeployTab wallet={wallet} />}
            {tab === 'interact' && <InteractTab wallet={wallet} />}
          </div>
        </div>
      </div>
    </section>
  );
}

function NetworkTab({ wallet }: Props) {
  const params = [
    { label: 'Network Name', value: NETWORK_NAME },
    { label: 'Chain ID', value: `${CHAIN_ID} (${CHAIN_ID_HEX})` },
    { label: 'Currency', value: `${TICKER} (${DECIMALS} decimals)` },
    { label: 'RPC URL', value: RPC_PUBLIC },
    { label: 'Explorer URL', value: EXPLORER_URL },
  ];

  return (
    <div className="network-tab">
      <p className="tab-desc">Add IronDAG to MetaMask or any EVM-compatible wallet.</p>
      <div className="network-params">
        {params.map(p => (
          <div key={p.label} className="network-param">
            <span className="np-label">{p.label}</span>
            <span
              className="np-value text-mono"
              onClick={() => navigator.clipboard.writeText(p.value).catch(() => {})}
              title="Click to copy"
            >
              {p.value}
            </span>
          </div>
        ))}
      </div>
      <button className="action-btn" onClick={wallet.addNetwork}>
        <i className="fas fa-plus-circle" /> Add to MetaMask
      </button>
      {!wallet.connected && (
        <button className="action-btn secondary" onClick={wallet.connect} style={{ marginLeft: '0.75rem' }}>
          <i className="fas fa-wallet" /> Connect Wallet
        </button>
      )}
    </div>
  );
}

function SendTab({ wallet }: Props) {
  const [to, setTo] = useState('');
  const [amount, setAmount] = useState('');
  const [status, setStatus] = useState<'idle' | 'pending' | 'ok' | 'err'>('idle');
  const [result, setResult] = useState('');
  const [balance, setBalance] = useState<string | null>(null);

  async function fetchBalance() {
    if (!wallet.address) return;
    try {
      const balHex = await rpcCall<string>('eth_getBalance', [wallet.address, 'latest']);
      setBalance(fmtWei(balHex));
    } catch { /* */ }
  }

  async function send() {
    if (!wallet.provider) { setStatus('err'); setResult('Connect wallet first'); return; }
    if (!/^0x[0-9a-fA-F]{40}$/.test(to)) { setStatus('err'); setResult('Invalid address'); return; }
    if (!amount || isNaN(parseFloat(amount)) || parseFloat(amount) <= 0) { setStatus('err'); setResult('Invalid amount'); return; }
    setStatus('pending'); setResult('');
    try {
      const hash = await sendIDAG(wallet.provider, to, amount);
      setStatus('ok'); setResult(`Tx sent: ${hash}`);
    } catch (e: unknown) {
      setStatus('err'); setResult((e as Error).message ?? 'Send failed');
    }
  }

  return (
    <div className="send-tab">
      <p className="tab-desc">Send {TICKER} to any address.</p>
      {wallet.connected ? (
        <>
          <div className="wallet-info">
            <span className="text-mono">{wallet.address}</span>
            <button className="inline-btn" onClick={fetchBalance}><i className="fas fa-sync-alt" /></button>
            {balance && <span className="balance">{balance}</span>}
          </div>
          <div className="form-group">
            <label>To Address</label>
            <input className="form-input" placeholder="0x…" value={to} onChange={e => setTo(e.target.value)} />
          </div>
          <div className="form-group">
            <label>Amount ({TICKER})</label>
            <input className="form-input" placeholder="0.1" type="number" min="0" value={amount} onChange={e => setAmount(e.target.value)} />
          </div>
          <button className="action-btn" onClick={send} disabled={status === 'pending'}>
            {status === 'pending' ? <><i className="fas fa-spinner fa-spin" /> Sending…</> : `Send ${TICKER}`}
          </button>
          {result && <div className={`result-box ${status}`}>{result}</div>}
        </>
      ) : (
        <button className="action-btn" onClick={wallet.connect}>
          <i className="fas fa-wallet" /> Connect Wallet to Send
        </button>
      )}
    </div>
  );
}

function DeployTab({ wallet }: Props) {
  const [bytecode, setBytecode] = useState('');
  const [abi, setAbi] = useState('[]');
  const [args, setArgs] = useState('');
  const [status, setStatus] = useState<'idle' | 'pending' | 'ok' | 'err'>('idle');
  const [result, setResult] = useState('');

  async function deploy() {
    if (!wallet.provider) { setStatus('err'); setResult('Connect wallet first'); return; }
    let parsedAbi: object[] = [];
    try { parsedAbi = JSON.parse(abi); } catch { setStatus('err'); setResult('Invalid ABI JSON'); return; }
    let parsedArgs: unknown[] = [];
    if (args.trim()) { try { parsedArgs = JSON.parse(args); } catch { setStatus('err'); setResult('Invalid constructor args JSON'); return; } }

    setStatus('pending'); setResult('Deploying…');
    try {
      const addr = await deployContract(wallet.provider, bytecode.trim(), parsedAbi, parsedArgs);
      setStatus('ok'); setResult(`Deployed at: ${addr}`);
    } catch (e: unknown) {
      setStatus('err'); setResult((e as Error).message ?? 'Deploy failed');
    }
  }

  return (
    <div className="deploy-tab">
      <p className="tab-desc">Deploy an EVM smart contract to IronDAG.</p>
      {wallet.connected ? (
        <>
          <div className="form-group">
            <label>Bytecode (hex, with 0x)</label>
            <textarea className="form-textarea" placeholder="0x608060…" value={bytecode} onChange={e => setBytecode(e.target.value)} rows={3} />
          </div>
          <div className="form-group">
            <label>ABI (JSON array)</label>
            <textarea className="form-textarea" placeholder='[{"type":"constructor","inputs":[…],"stateMutability":"nonpayable"}]' value={abi} onChange={e => setAbi(e.target.value)} rows={4} />
          </div>
          <div className="form-group">
            <label>Constructor Args (JSON array, optional)</label>
            <input className="form-input" placeholder='["arg1", 42]' value={args} onChange={e => setArgs(e.target.value)} />
          </div>
          <button className="action-btn" onClick={deploy} disabled={status === 'pending'}>
            {status === 'pending' ? <><i className="fas fa-spinner fa-spin" /> Deploying…</> : 'Deploy Contract'}
          </button>
          {result && <div className={`result-box ${status}`}>{result}</div>}
        </>
      ) : (
        <button className="action-btn" onClick={wallet.connect}>
          <i className="fas fa-wallet" /> Connect Wallet to Deploy
        </button>
      )}
    </div>
  );
}

function InteractTab({ wallet }: Props) {
  const [contractAddr, setContractAddr] = useState('');
  const [abi, setAbi] = useState('[]');
  const [method, setMethod] = useState('');
  const [callArgs, setCallArgs] = useState('');
  const [status, setStatus] = useState<'idle' | 'pending' | 'ok' | 'err'>('idle');
  const [result, setResult] = useState('');
  const [methods, setMethods] = useState<{ name: string; stateMutability: string }[]>([]);

  function parseAbi() {
    try {
      const arr = JSON.parse(abi) as { type?: string; name?: string; stateMutability?: string }[];
      const fns = arr.filter(e => e.type === 'function' && e.name).map(e => ({ name: e.name!, stateMutability: e.stateMutability ?? 'nonpayable' }));
      setMethods(fns);
    } catch { setStatus('err'); setResult('Invalid ABI'); }
  }

  async function callMethod() {
    if (!wallet.provider) { setStatus('err'); setResult('Connect wallet first'); return; }
    if (!/^0x[0-9a-fA-F]{40}$/.test(contractAddr)) { setStatus('err'); setResult('Invalid contract address'); return; }
    let parsedAbi: object[] = [];
    try { parsedAbi = JSON.parse(abi); } catch { setStatus('err'); setResult('Invalid ABI'); return; }
    let parsedArgs: unknown[] = [];
    if (callArgs.trim()) { try { parsedArgs = JSON.parse(callArgs); } catch { setStatus('err'); setResult('Invalid args JSON'); return; } }

    setStatus('pending'); setResult('');
    try {
      const { ethers } = await import('ethers');
      const selectedMethod = methods.find(m => m.name === method);
      const isView = selectedMethod?.stateMutability === 'view' || selectedMethod?.stateMutability === 'pure';
      const contract = new ethers.Contract(contractAddr, parsedAbi, isView ? await wallet.provider.getSigner() : await wallet.provider.getSigner());
      const fn = contract[method];
      if (!fn) throw new Error(`Method "${method}" not found`);
      const res = await fn(...parsedArgs);
      setStatus('ok');
      setResult(typeof res === 'object' ? JSON.stringify(res, null, 2) : String(res));
    } catch (e: unknown) {
      setStatus('err'); setResult((e as Error).message ?? 'Call failed');
    }
  }

  return (
    <div className="interact-tab">
      <p className="tab-desc">Call read or write functions on any IronDAG contract.</p>
      <div className="form-group">
        <label>Contract Address</label>
        <input className="form-input" placeholder="0x…" value={contractAddr} onChange={e => setContractAddr(e.target.value)} />
      </div>
      <div className="form-group">
        <label>ABI</label>
        <div className="abi-row">
          <textarea className="form-textarea" placeholder='[{"type":"function","name":"balanceOf","inputs":[…],"outputs":[…],"stateMutability":"view"}]' value={abi} onChange={e => setAbi(e.target.value)} rows={3} />
          <button className="inline-btn" onClick={parseAbi}>Parse</button>
        </div>
      </div>
      {methods.length > 0 && (
        <div className="form-group">
          <label>Method</label>
          <select className="form-select" value={method} onChange={e => setMethod(e.target.value)}>
            <option value="">Select method</option>
            {methods.map(m => <option key={m.name} value={m.name}>{m.name} ({m.stateMutability})</option>)}
          </select>
        </div>
      )}
      <div className="form-group">
        <label>Arguments (JSON array, optional)</label>
        <input className="form-input" placeholder='["0xabc…", 100]' value={callArgs} onChange={e => setCallArgs(e.target.value)} />
      </div>
      <button className="action-btn" onClick={callMethod} disabled={!method || status === 'pending'}>
        {status === 'pending' ? <><i className="fas fa-spinner fa-spin" /> Calling…</> : 'Call Method'}
      </button>
      {result && <div className={`result-box ${status} result-mono`}>{result}</div>}
    </div>
  );
}
