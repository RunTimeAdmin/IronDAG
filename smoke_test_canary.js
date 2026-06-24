#!/usr/bin/env node
const http = require('http');

function rpcCall(method, params = []) {
    return new Promise((resolve, reject) => {
        const postData = JSON.stringify({
            jsonrpc: '2.0',
            id: 1,
            method,
            params
        });

        const options = {
            hostname: 'localhost',
            port: 8545,
            path: '/',
            method: 'POST',
            headers: {
                'Content-Type': 'application/json',
                'Content-Length': Buffer.byteLength(postData)
            }
        };

        const req = http.request(options, (res) => {
            let data = '';
            res.on('data', chunk => data += chunk);
            res.on('end', () => {
                try {
                    const response = JSON.parse(data);
                    if (response.error) {
                        reject(new Error(`RPC Error: ${JSON.stringify(response.error)}`));
                    } else {
                        resolve(response.result);
                    }
                } catch (e) {
                    reject(e);
                }
            });
        });

        req.on('error', reject);
        req.write(postData);
        req.end();
    });
}

async function main() {
    const contractAddr = '0x1000000000000000000000000000000000000001';
    
    console.log('🧪 SMOKE TEST - EVM Storage Persistence');
    console.log('=========================================\n');
    
    // Call setValue(999)
    console.log('Step 1: Calling setValue(999)...');
    const setValueCalldata = '0x60fe47b1' + (999).toString(16).padStart(64, '0');
    
    try {
        const txHash = await rpcCall('eth_sendRawTransaction', [{
            from: '0x0000000000000000000000000000000000000001',
            to: contractAddr,
            data: setValueCalldata,
            gas: '0x100000',
            gasPrice: '0x1'
        }]);
        console.log('✅ setValue(999) transaction sent:', txHash);
    } catch (error) {
        console.error('❌ setValue failed:', error.message);
        process.exit(1);
    }
    
    // Wait for transaction to be mined
    console.log('\nStep 2: Waiting for transaction to be mined...');
    await new Promise(resolve => setTimeout(resolve, 3000));
    
    // Call getValue()
    console.log('\nStep 3: Calling getValue()...');
    const getValueCalldata = '0x20965255';
    
    try {
        const result = await rpcCall('eth_call', [{
            to: contractAddr,
            data: getValueCalldata
        }, 'latest']);
        
        const value = parseInt(result, 16);
        console.log('✅ getValue() returned:', value);
        
        if (value === 999) {
            console.log('\n🟢 BEFORE RESTART: Value persisted correctly (999)');
        } else {
            console.log(`\n🔴 BEFORE RESTART: Value incorrect (expected 999, got ${value})`);
        }
    } catch (error) {
        console.error('❌ getValue failed:', error.message);
        process.exit(1);
    }
    
    console.log('\n✅ Pre-restart test complete');
    console.log('Next: Kill node, restart, and call getValue() again');
}

main().catch(console.error);
