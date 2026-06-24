import { WalletState } from '../hooks/useWallet';
import { fmtAddr } from '../lib/format';

interface Props {
  wallet: WalletState;
  onSearch: (q: string) => void;
}

export function Header({ wallet, onSearch }: Props) {
  return (
    <header className="header">
      <div className="header-content">
        <div className="logo-section">
          <a href="/" className="logo">
            <img src="/irondag-logo.png" alt="IronDAG" className="logo-img" onError={e => (e.currentTarget.style.display = 'none')} />
            <span className="logo-text">IronDAG</span>
          </a>
        </div>

        <nav className="main-nav">
          <a href="#" className="nav-link active">Home</a>
          <a href="#blocks" className="nav-link">Blocks</a>
          <a href="#transactions" className="nav-link">Transactions</a>
          <a href="#dev-tools" className="nav-link">Dev Tools</a>
          <a href="#faucet" className="nav-link">Faucet</a>
        </nav>

        <div className="header-right">
          {wallet.connected ? (
            <div className="wallet-connected">
              <span
                className="wallet-address"
                title={wallet.address ?? ''}
                onClick={() => wallet.address && navigator.clipboard.writeText(wallet.address)}
              >
                {fmtAddr(wallet.address ?? '')}
              </span>
              <button className="disconnect-btn" onClick={wallet.disconnect} title="Disconnect">
                <i className="fas fa-sign-out-alt" />
              </button>
            </div>
          ) : (
            <button className="connect-wallet-btn" onClick={wallet.connect}>
              <i className="fas fa-wallet" /> Connect Wallet
            </button>
          )}
        </div>
      </div>
    </header>
  );
}
