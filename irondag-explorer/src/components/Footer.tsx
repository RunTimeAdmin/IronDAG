import { CHAIN_ID, TICKER, NETWORK_NAME, RPC_PUBLIC } from '../lib/config';

export function Footer() {
  return (
    <footer className="footer">
      <div className="container">
        <div className="footer-grid">
          <div className="footer-col">
            <h3 className="footer-heading">IronDAG Explorer</h3>
            <p className="footer-text">Real-time GhostDAG block explorer for the IronDAG network with BraidCore dual-stream PoW.</p>
            <div className="footer-chain-info">
              <span>{NETWORK_NAME}</span>
              <span>Chain ID {CHAIN_ID}</span>
            </div>
          </div>

          <div className="footer-col">
            <h3 className="footer-heading">Network</h3>
            <ul className="footer-links">
              <li><a href="#" className="footer-link">Explorer</a></li>
              <li><a href="#faucet" className="footer-link">{TICKER} Faucet</a></li>
              <li><a href="#dev-tools" className="footer-link">Dev Tools</a></li>
            </ul>
          </div>

          <div className="footer-col">
            <h3 className="footer-heading">Resources</h3>
            <ul className="footer-links">
              <li>
                <span
                  className="footer-link copyable"
                  onClick={() => navigator.clipboard.writeText(RPC_PUBLIC).catch(() => {})}
                  title="Click to copy RPC URL"
                >
                  RPC: {RPC_PUBLIC}
                </span>
              </li>
              <li>
                <span
                  className="footer-link copyable"
                  onClick={() => navigator.clipboard.writeText(String(CHAIN_ID)).catch(() => {})}
                  title="Click to copy Chain ID"
                >
                  Chain ID: {CHAIN_ID}
                </span>
              </li>
            </ul>
          </div>
        </div>

        <div className="footer-bottom">
          <span>© {new Date().getFullYear()} IronDAG · GhostDAG Consensus · BraidCore Dual-Stream PoW</span>
          <span>{TICKER} · {CHAIN_ID}</span>
        </div>
      </div>
    </footer>
  );
}
