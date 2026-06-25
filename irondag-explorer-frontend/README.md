# IronDAG Block Explorer - Frontend

A modern, responsive web interface for exploring the IronDAG blockchain.

## Features

- **Network Dashboard** - Real-time network statistics
- **Block Viewer** - Browse recent blocks with details
- **Transaction Viewer** - View recent transactions
- **Address Lookup** - Search and view address information
- **Search** - Search by block hash, transaction hash, or address
- **Auto-refresh** - Dashboard updates every 30 seconds

## Production (explorer.irondag.io)

**Status**: LIVE (Apr 2026) at https://explorer.irondag.io

For the live site, nginx must **proxy /rpc** to the testnet node or the explorer will load but data will not (RPC calls will 404). Add to your nginx server block:

```nginx
location /rpc {
    # srv1296980 (<vps-ip>) - miner node
    proxy_pass https://rpc.irondag.io;
    proxy_http_version 1.1;
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
}
```

Then reload nginx: `sudo nginx -t && sudo systemctl reload nginx`. Ensure the logo file `irondag-logo.png` is in the same directory as `index.html` (e.g. copied from `brand-assets/logos/irondag_project_logo.png`).

### Faucet

Testnet faucet enabled — mints 10 IDAG per request. Access via the explorer UI.

## Known Issues

- **CORS wildcard on .31**: The RPC endpoint currently returns `Access-Control-Allow-Origin: *`. This should be restricted to the explorer domain before mainnet.

## Setup

1. **Start the IronDAG node:**
   ```bash
   cd irondag-blockchain
   cargo run --bin node
   ```

2. **Open the frontend:**
   - Simply open `index.html` in a web browser
   - Or use a local web server:
     ```bash
     # Python 3
     python3 -m http.server 3000
     
     # Or Node.js
     npx http-server -p 3000
     ```

3. **Access the explorer:**
   - Open `http://localhost:3000` in your browser
   - The explorer will connect to the API at `http://localhost:8546`

## API Endpoints Used

- `GET /api/stats/network` - Network statistics
- `GET /api/stats/chain` - Chain statistics
- `GET /api/blocks/recent` - Recent blocks
- `GET /api/transactions/recent` - Recent transactions
- `GET /api/blocks/:identifier` - Block details
- `GET /api/transactions/:hash` - Transaction details
- `GET /api/addresses/:address` - Address details
- `GET /api/search?q=...` - Search

## Configuration

To change the API endpoint, edit `app.js`:

```javascript
const API_BASE = 'http://localhost:8546/api';
```

## Browser Support

- Chrome/Edge (latest)
- Firefox (latest)
- Safari (latest)

## Future Enhancements

- Real-time WebSocket updates
- Transaction history pagination
- Block detail pages
- Transaction detail pages
- Address transaction history
- Charts and graphs
- Export functionality

