import { useState } from 'react';
import { rpcCall } from '../lib/rpc';
import { TICKER, CHAIN_ID } from '../lib/config';
import { fmtHash } from '../lib/format';
import { WalletState } from '../hooks/useWallet';

interface Props { wallet: WalletState }

const FAUCET_AMOUNT = '1';
const COOLDOWN_MS = 24 * 60 * 60 * 1000;

export function Faucet({ wallet }: Props) {
  const [address, setAddress] = useState('');
  const [status, setStatus] = useState<'idle' | 'pending' | 'ok' | 'err'>('idle');
  const [txHash, setTxHash] = useState('');
  const [message, setMessage] = useState('');

  async function claim() {
    const addr = (wallet.address ?? address).trim();
    if (!/^0x[0-9a-fA-F]{40}$/.test(addr)) {
      setStatus('err'); setMessage('Enter a valid 0x address'); return;
    }

    // Rate-limit check (browser-side, not a real server-side guard)
    const key = `faucet_last_${addr.toLowerCase()}`;
    const last = parseInt(localStorage.getItem(key) ?? '0', 10);
    if (Date.now() - last < COOLDOWN_MS) {
      const left = Math.ceil((last + COOLDOWN_MS - Date.now()) / 3600000);
      setStatus('err'); setMessage(`Already claimed. Try again in ${left}h.`); return;
    }

    setStatus('pending'); setMessage('');
    try {
      const result = await rpcCall<{ txHash?: string; error?: string }>('irondag_faucetDrip', [addr, FAUCET_AMOUNT]);
      if (result?.txHash) {
        localStorage.setItem(key, Date.now().toString());
        setTxHash(result.txHash);
        setStatus('ok');
        setMessage(`${FAUCET_AMOUNT} ${TICKER} sent!`);
      } else {
        setStatus('err');
        setMessage(result?.error ?? 'Faucet error — try again');
      }
    } catch (e: unknown) {
      setStatus('err');
      setMessage((e as Error).message ?? 'Request failed');
    }
  }

  return (
    <section id="faucet" className="faucet-section">
      <div className="container">
        <div className="faucet-card">
          <h2><i className="fas fa-faucet" /> {TICKER} Faucet</h2>
          <p className="faucet-subtitle">Get {FAUCET_AMOUNT} {TICKER} for testing on Chain ID {CHAIN_ID}</p>

          <div className="faucet-form">
            {!wallet.connected ? (
              <>
                <input
                  type="text"
                  className="faucet-input"
                  placeholder="0x… address"
                  value={address}
                  onChange={e => setAddress(e.target.value)}
                />
                <button className="faucet-btn" onClick={claim} disabled={status === 'pending'}>
                  {status === 'pending' ? <><i className="fas fa-spinner fa-spin" /> Sending…</> : <>Request {TICKER}</>}
                </button>
                <button className="wallet-btn-secondary" onClick={wallet.connect}>
                  <i className="fas fa-wallet" /> Connect wallet instead
                </button>
              </>
            ) : (
              <>
                <div className="faucet-wallet-addr">
                  Connected: <span className="text-mono">{wallet.address}</span>
                </div>
                <button className="faucet-btn" onClick={claim} disabled={status === 'pending'}>
                  {status === 'pending' ? <><i className="fas fa-spinner fa-spin" /> Sending…</> : <>Request {TICKER}</>}
                </button>
              </>
            )}
          </div>

          {status === 'ok' && (
            <div className="faucet-result faucet-ok">
              <i className="fas fa-check-circle" /> {message}
              {txHash && <div className="text-mono text-sm">{fmtHash(txHash, 12, 8)}</div>}
            </div>
          )}
          {status === 'err' && (
            <div className="faucet-result faucet-err">
              <i className="fas fa-times-circle" /> {message}
            </div>
          )}

          <div className="faucet-info">
            <span><i className="fas fa-clock" /> 1 drip per 24h per address</span>
            <span><i className="fas fa-coins" /> {FAUCET_AMOUNT} {TICKER} per request</span>
            <span><i className="fas fa-link" /> Chain ID {CHAIN_ID}</span>
          </div>
        </div>
      </div>
    </section>
  );
}
