import { useState } from 'react';
import { Header } from './components/Header';
import { Hero } from './components/Hero';
import { StatsSummary } from './components/StatsSummary';
import { DAGVisualizer } from './components/DAGVisualizer';
import { BlockList } from './components/BlockList';
import { TransactionList } from './components/TransactionList';
import { DetailPanel } from './components/DetailPanel';
import { Faucet } from './components/Faucet';
import { DevTools } from './components/DevTools';
import { Footer } from './components/Footer';
import { useChainStats } from './hooks/useChainStats';
import { useWallet } from './hooks/useWallet';

export default function App() {
  const stats = useChainStats();
  const wallet = useWallet();
  const [detailQuery, setDetailQuery] = useState<string | null>(null);

  function handleSearch(q: string) {
    setDetailQuery(q);
    setTimeout(() => {
      document.getElementById('detail-panel')?.scrollIntoView({ behavior: 'smooth', block: 'start' });
    }, 100);
  }

  return (
    <>
      <Header wallet={wallet} onSearch={handleSearch} />
      <main>
        <Hero onSearch={handleSearch} />
        <StatsSummary stats={stats} />
        <DAGVisualizer onBlockClick={setDetailQuery} />
        {detailQuery && (
          <DetailPanel query={detailQuery} onClose={() => setDetailQuery(null)} />
        )}
        <div className="lists-section">
          <div className="container">
            <div className="lists-grid">
              <BlockList onBlockClick={setDetailQuery} />
              <TransactionList onTxClick={setDetailQuery} />
            </div>
          </div>
        </div>
        <Faucet wallet={wallet} />
        <DevTools wallet={wallet} />
      </main>
      <Footer />
    </>
  );
}
