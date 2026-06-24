import { useState } from 'react';

interface Props {
  onSearch: (q: string) => void;
}

export function Hero({ onSearch }: Props) {
  const [query, setQuery] = useState('');

  function submit() {
    const q = query.trim();
    if (q) { onSearch(q); setQuery(''); }
  }

  return (
    <section className="hero">
      <div className="hero-content">
        <div className="hero-logo">
          <img src="/irondag-logo.png" alt="IronDAG" className="hero-logo-img" onError={e => (e.currentTarget.style.display = 'none')} />
        </div>
        <h1>IronDAG Explorer</h1>
        <p className="hero-subtitle">Mainnet · Chain ID 11567</p>
        <div className="search-container">
          <i className="fas fa-search search-icon" />
          <input
            type="text"
            className="search-input"
            placeholder="Search by Address / Txn Hash / Block"
            value={query}
            onChange={e => setQuery(e.target.value)}
            onKeyDown={e => e.key === 'Enter' && submit()}
          />
          <button className="search-btn" onClick={submit}>
            <i className="fas fa-search" />
          </button>
        </div>
      </div>
    </section>
  );
}
