/**
 * IronDAG Performance Profiler
 * Measures RPC latency, block production rate, and system throughput
 */
const http = require('http');

const RPC_PORT = 8545;
let requestId = 1;

function rpc(method, params = []) {
  const start = Date.now();
  return new Promise((resolve, reject) => {
    const data = JSON.stringify({ jsonrpc: '2.0', method, params, id: requestId++ });
    const req = http.request({
      hostname: '127.0.0.1',
      port: RPC_PORT,
      method: 'POST',
      headers: { 'Content-Type': 'application/json' }
    }, res => {
      let body = '';
      res.on('data', chunk => body += chunk);
      res.on('end', () => {
        const latency = Date.now() - start;
        try {
          const result = JSON.parse(body);
          resolve({ result, latency });
        } catch (e) {
          reject(new Error(`Invalid JSON: ${body}`));
        }
      });
    });
    req.on('error', reject);
    req.setTimeout(5000, () => reject(new Error('Request timeout')));
    req.write(data);
    req.end();
  });
}

async function measureRpcLatency(method, params, iterations) {
  const latencies = [];
  let errors = 0;
  
  for (let i = 0; i < iterations; i++) {
    try {
      const { latency } = await rpc(method, params);
      latencies.push(latency);
    } catch (e) {
      errors++;
    }
  }
  
  if (latencies.length === 0) {
    return { avg: 0, min: 0, max: 0, p95: 0, errors };
  }
  
  latencies.sort((a, b) => a - b);
  const avg = latencies.reduce((a, b) => a + b, 0) / latencies.length;
  const p95 = latencies[Math.floor(latencies.length * 0.95)];
  
  return {
    avg: avg.toFixed(2),
    min: latencies[0],
    max: latencies[latencies.length - 1],
    p95,
    errors
  };
}

async function measureBlockRate(durationSec) {
  const { result: startResult } = await rpc('eth_blockNumber');
  const startBlock = parseInt(startResult.result, 16);
  
  await new Promise(r => setTimeout(r, durationSec * 1000));
  
  const { result: endResult } = await rpc('eth_blockNumber');
  const endBlock = parseInt(endResult.result, 16);
  
  const blocksProduced = endBlock - startBlock;
  const blocksPerSec = blocksProduced / durationSec;
  
  return { blocksProduced, blocksPerSec, startBlock, endBlock };
}

async function runProfile() {
  console.log('╔═══════════════════════════════════════════════════════════╗');
  console.log('║      IronDAG Performance Profiler                      ║');
  console.log('╚═══════════════════════════════════════════════════════════╝\n');

  // Test connectivity
  console.log('Testing RPC connectivity...');
  try {
    const { result } = await rpc('eth_chainId');
    console.log(`  Chain ID: ${result.result}\n`);
  } catch (e) {
    console.error('Failed to connect to RPC:', e.message);
    process.exit(1);
  }

  // RPC Latency Tests
  console.log('═══════════════════════════════════════════════════════════');
  console.log('RPC LATENCY BENCHMARKS (100 iterations each)\n');

  const tests = [
    { name: 'eth_blockNumber', method: 'eth_blockNumber', params: [] },
    { name: 'eth_chainId', method: 'eth_chainId', params: [] },
    { name: 'eth_gasPrice', method: 'eth_gasPrice', params: [] },
    { name: 'eth_getBalance', method: 'eth_getBalance', params: ['0x7e5f4552091a69125d5dfcb7b8c2659029395bdf', 'latest'] },
    { name: 'net_peerCount', method: 'net_peerCount', params: [] },
  ];

  for (const test of tests) {
    const stats = await measureRpcLatency(test.method, test.params, 100);
    console.log(`  ${test.name.padEnd(20)} avg: ${stats.avg}ms  min: ${stats.min}ms  max: ${stats.max}ms  p95: ${stats.p95}ms  errors: ${stats.errors}`);
  }

  // Block Production Rate
  console.log('\n═══════════════════════════════════════════════════════════');
  console.log('BLOCK PRODUCTION RATE (30 second measurement)\n');

  console.log('  Measuring block production...');
  const blockStats = await measureBlockRate(30);
  console.log(`  Blocks Produced: ${blockStats.blocksProduced}`);
  console.log(`  Blocks/Second:   ${blockStats.blocksPerSec.toFixed(2)}`);
  console.log(`  Start Block:     ${blockStats.startBlock}`);
  console.log(`  End Block:       ${blockStats.endBlock}`);

  // Concurrent RPC Load
  console.log('\n═══════════════════════════════════════════════════════════');
  console.log('CONCURRENT RPC LOAD TEST\n');

  const concurrentLevels = [1, 10, 50, 100];
  
  for (const concurrent of concurrentLevels) {
    const start = Date.now();
    const promises = [];
    
    for (let i = 0; i < concurrent; i++) {
      promises.push(rpc('eth_blockNumber').catch(() => null));
    }
    
    const results = await Promise.all(promises);
    const duration = Date.now() - start;
    const successful = results.filter(r => r !== null).length;
    const rps = (successful / (duration / 1000)).toFixed(0);
    
    console.log(`  ${concurrent.toString().padStart(3)} concurrent:  ${duration}ms  success: ${successful}/${concurrent}  RPS: ${rps}`);
  }

  // Summary
  console.log('\n═══════════════════════════════════════════════════════════');
  console.log('PERFORMANCE SUMMARY\n');
  
  // Get current metrics
  try {
    const { result: blockNum } = await rpc('eth_blockNumber');
    const currentBlock = parseInt(blockNum.result, 16);
    
    const { result: balance } = await rpc('eth_getBalance', ['0x0101010101010101010101010101010101010101', 'latest']);
    const minerBalance = parseInt(balance.result, 16);
    
    console.log(`  Current Block Height: ${currentBlock}`);
    console.log(`  Miner Balance:        ${minerBalance} wei (${(minerBalance / 1e18).toFixed(2)} IDAG)`);
    console.log(`  Block Rate:           ${blockStats.blocksPerSec.toFixed(2)} blocks/sec`);
    console.log(`  Dev Difficulty:       Active (fast block times)`);
  } catch (e) {
    console.log('  Could not fetch summary metrics');
  }

  console.log('\n═══════════════════════════════════════════════════════════');
  console.log('NOTE: Transaction throughput requires signed transactions.');
  console.log('Use deploy_contract.js for EVM transaction benchmarks.');
  console.log('═══════════════════════════════════════════════════════════\n');
}

runProfile().catch(console.error);
