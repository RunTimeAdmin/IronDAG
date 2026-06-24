const https = require('https');

const RPC_URL = 'https://explorer.irondag.io/rpc';

function rpcCall(method, params) {
    return new Promise((resolve, reject) => {
        const data = JSON.stringify({
            jsonrpc: '2.0',
            method: method,
            params: params,
            id: 1
        });

        const url = new URL(RPC_URL);
        const options = {
            hostname: url.hostname,
            port: 443,
            path: url.pathname,
            method: 'POST',
            headers: {
                'Content-Type': 'application/json',
                'Content-Length': data.length
            }
        };

        const req = https.request(options, (res) => {
            let body = '';
            res.on('data', chunk => body += chunk);
            res.on('end', () => {
                try {
                    const json = JSON.parse(body);
                    resolve(json.result);
                } catch (e) {
                    reject(e);
                }
            });
        });

        req.on('error', reject);
        req.setTimeout(10000, () => reject(new Error('Timeout')));
        req.write(data);
        req.end();
    });
}

async function main() {
    console.log('Checking testnet balances...\n');
    
    // Check block number
    const blockNum = await rpcCall('eth_blockNumber', []);
    console.log('Current block:', parseInt(blockNum, 16));
    
    // Addresses to check
    const addresses = [
        { name: 'Hardcoded miner [1u8;20]', addr: '0x0101010101010101010101010101010101010101' },
        { name: 'Zero address', addr: '0x0000000000000000000000000000000000000000' },
        { name: 'Test key 0x01 owner', addr: '0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf' },
        { name: 'Wiki test account', addr: '0x742d35Cc6634C0532925a3b844Bc9e7595f8dE1B' },
    ];
    
    for (const {name, addr} of addresses) {
        const balance = await rpcCall('eth_getBalance', [addr, 'latest']);
        const balanceEth = parseInt(balance || '0x0', 16) / 1e18;
        console.log(`${name}:`);
        console.log(`  Address: ${addr}`);
        console.log(`  Balance: ${balanceEth} IDAG\n`);
    }
    
    // Check latest block miner
    const block = await rpcCall('eth_getBlockByNumber', ['latest', false]);
    console.log('Latest block miner:', block.miner);
}

main().catch(console.error);
