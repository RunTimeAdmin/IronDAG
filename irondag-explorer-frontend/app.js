// IronDAG Block Explorer - Official Brand Frontend
// Production-Grade Blockchain Explorer UI

// ============================================================================
// CONFIGURATION
// ============================================================================

const urlParams = new URLSearchParams(window.location.search);
// When on explorer domain, use /rpc (nginx should proxy to testnet). Else use public testnet.
const isExplorerHost = window.location.hostname === 'explorer.irondag.io' || window.location.hostname === 'localhost';
const DEFAULT_RPC = isExplorerHost ? '/rpc' : 'http://76.13.101.31:8545';
const RPC_BASE = urlParams.get('rpc') || DEFAULT_RPC;

// Cache for DAG health to avoid showing 0% on RPC failures
let lastKnownHealth = null;
let lastHealthUpdateTime = 0;
const HEALTH_STALE_TTL = 60000; // 60 seconds before marking stale

// Cache for dagStats to degrade gracefully on RPC timeouts
let lastDagStats = null;

// Cache for block number and finalized to avoid flicker on RPC timeouts
let lastBlockNumber = null;
let lastFinalizedDisplay = null;

// Cache for peer count to avoid showing 0 on RPC timeouts
let lastKnownPeerCount = null;
let lastPeerCountUpdateTime = 0;

// Clear any old cached RPC endpoints
localStorage.removeItem('rpc_endpoint');
console.log('Explorer connecting to RPC:', RPC_BASE);

// ============================================================================
// UTILITY FUNCTIONS
// ============================================================================

/**
 * Format timestamp to relative time (e.g., "5m ago", "2h ago")
 * Shows seconds when < 10m for livelier display
 */
function timeAgo(timestamp) {
    const now = Math.floor(Date.now() / 1000);
    const diff = now - timestamp;
    
    if (diff < 60) return `${diff}s ago`;
    if (diff < 600) return `${Math.floor(diff / 60)}m ${diff % 60}s ago`;
    if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
    if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
    if (diff < 604800) return `${Math.floor(diff / 86400)}d ago`;
    return new Date(timestamp * 1000).toLocaleDateString();
}

/**
 * Live-update all relative time displays (call every second)
 */
function updateLiveTimes() {
    document.querySelectorAll('[data-timestamp]').forEach(el => {
        const ts = parseInt(el.dataset.timestamp, 10);
        if (!isNaN(ts)) el.textContent = timeAgo(ts);
    });
}

/**
 * Truncate address for display (0x1234...5678)
 */
function truncateAddress(address, startChars = 6, endChars = 4) {
    if (!address || address.length < startChars + endChars + 3) return address;
    return `${address.slice(0, startChars)}...${address.slice(-endChars)}`;
}

/**
 * Truncate hash for display
 */
function truncateHash(hash, startChars = 10, endChars = 4) {
    if (!hash || hash.length < startChars + endChars + 3) return hash;
    return `${hash.slice(0, startChars)}...${hash.slice(-endChars)}`;
}

/**
 * Format number with commas
 */
function formatNumber(num) {
    return num.toString().replace(/\B(?=(\d{3})+(?!\d))/g, ",");
}

/**
 * Detect transaction type based on data field
 */
function getTransactionType(tx) {
    if (!tx.to || tx.to === '0x0000000000000000000000000000000000000000') {
        return 'contract-create';
    }
    if (tx.input && tx.input !== '0x' && tx.input.length > 2) {
        return 'contract-call';
    }
    return 'coin-transfer';
}

/**
 * Get badge HTML for transaction type
 */
function getTxTypeBadge(type) {
    const badges = {
        'contract-call': '<span class="tx-type-badge contract-call">CONTRACT CALL</span>',
        'contract-create': '<span class="tx-type-badge contract-call">CONTRACT CREATE</span>',
        'coin-transfer': '<span class="tx-type-badge coin-transfer">COIN TRANSFER</span>'
    };
    return badges[type] || badges['coin-transfer'];
}

/**
 * Get badge HTML for transaction status
 * @param {string} status - 'success', 'failed', or 'pending'
 */
function getTxStatusBadge(status) {
    const badges = {
        'success': '<span class="tx-status-badge success"><i class="fas fa-check"></i> SUCCESS</span>',
        'failed': '<span class="tx-status-badge failed"><i class="fas fa-times"></i> FAILED</span>',
        'pending': '<span class="tx-status-badge pending"><i class="fas fa-clock"></i> PENDING</span>'
    };
    return badges[status] || badges['pending'];
}

/**
 * Fetch transaction receipt and determine status
 * @param {string} txHash - Transaction hash
 * @returns {Promise<string>} - 'success', 'failed', or 'pending'
 */
async function getTransactionStatus(txHash) {
    try {
        const receipt = await rpcCall('eth_getTransactionReceipt', [txHash]);
        if (!receipt) return 'pending';
        if (receipt.status === '0x1') return 'success';
        if (receipt.status === '0x0') return 'failed';
        return 'pending';
    } catch (error) {
        console.warn(`Failed to get receipt for ${txHash}:`, error.message);
        return 'pending';
    }
}

/**
 * Copy text to clipboard
 */
async function copyToClipboard(text) {
    try {
        await navigator.clipboard.writeText(text);
        // Brief visual feedback could be added here
    } catch (err) {
        console.error('Failed to copy:', err);
    }
}

// ============================================================================
// RPC HELPER WITH RATE LIMITING
// ============================================================================

// Request queue for rate limiting - prevents RPC flooding
const rpcQueue = {
    pending: [],
    active: 0,
    maxConcurrent: 2,      // Max parallel RPC calls
    minInterval: 100,      // ms between requests
    lastRequestTime: 0,
    rateLimitBackoff: 0,   // Global backoff when rate limited

    async enqueue(fn) {
        return new Promise((resolve, reject) => {
            this.pending.push({ fn, resolve, reject });
            this.processNext();
        });
    },

    async processNext() {
        if (this.active >= this.maxConcurrent || this.pending.length === 0) return;

        // If globally rate limited, wait before processing
        if (this.rateLimitBackoff > Date.now()) {
            setTimeout(() => this.processNext(), this.rateLimitBackoff - Date.now() + 50);
            return;
        }

        const now = Date.now();
        const wait = Math.max(0, this.minInterval - (now - this.lastRequestTime));

        if (wait > 0) {
            setTimeout(() => this.processNext(), wait);
            return;
        }

        this.active++;
        this.lastRequestTime = Date.now();
        const { fn, resolve, reject } = this.pending.shift();

        try {
            const result = await fn();
            resolve(result);
        } catch (e) {
            // If rate limited, set global backoff and re-queue
            if (e.message && e.message.includes('Rate limit')) {
                this.rateLimitBackoff = Date.now() + 3000;
                // Re-queue this request
                this.pending.unshift({ fn, resolve, reject });
                this.active--;
                setTimeout(() => this.processNext(), 3000);
                return;
            }
            reject(e);
        } finally {
            if (this.active > 0) this.active--;
            this.processNext();
        }
    }
};

/**
 * @param {string} method
 * @param {any[]} params
 * @param {number | { retries?: number, quiet?: boolean }} [retriesOrOpts]  Legacy: retry count. Prefer `{ retries, quiet }`.
 */
async function rpcCall(method, params = [], retriesOrOpts = 3) {
    const opts = typeof retriesOrOpts === 'object' && retriesOrOpts !== null && !Array.isArray(retriesOrOpts)
        ? retriesOrOpts
        : { retries: retriesOrOpts };
    const retries = typeof opts.retries === 'number' && opts.retries > 0 ? opts.retries : 3;
    const quiet = Boolean(opts.quiet);

    return rpcQueue.enqueue(async () => {
        for (let attempt = 0; attempt < retries; attempt++) {
            try {
                const controller = new AbortController();
                const timeoutId = setTimeout(() => controller.abort(), 10000);

                const response = await fetch(RPC_BASE, {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({
                        jsonrpc: '2.0',
                        method: method,
                        params: params,
                        id: 1
                    }),
                    signal: controller.signal
                });

                clearTimeout(timeoutId);

                // Handle HTTP-level rate limiting
                if (response.status === 429 || response.status === 502) {
                    const delay = Math.min(2000 * Math.pow(2, attempt), 10000);
                    console.warn(`RPC rate limited (HTTP ${response.status}), backing off ${delay}ms...`);
                    rpcQueue.rateLimitBackoff = Date.now() + delay;
                    await new Promise(r => setTimeout(r, delay));
                    continue;
                }

                if (!response.ok) {
                    throw new Error(`HTTP error! status: ${response.status}`);
                }

                const data = await response.json();
                if (data.error) {
                    const errMsg = data.error.message || 'RPC error';
                    // Handle JSON-level rate limiting
                    if (errMsg.includes('Rate limit') || errMsg.includes('rate limit')) {
                        const delay = Math.min(2000 * Math.pow(2, attempt), 10000);
                        console.warn(`RPC rate limited (JSON), backing off ${delay}ms...`);
                        rpcQueue.rateLimitBackoff = Date.now() + delay;
                        await new Promise(r => setTimeout(r, delay));
                        continue;
                    }
                    // Node busy during mining — short backoff, silent retry
                    if (errMsg.includes('Node busy') || errMsg.includes('mining in progress')) {
                        if (attempt < retries - 1) {
                            await new Promise(r => setTimeout(r, 200));
                            continue;
                        }
                    }
                    throw new Error(errMsg);
                }

                return data.result;
            } catch (error) {
                if (error.name === 'AbortError') {
                    throw new Error('Request timed out');
                }
                // Check for rate limit in catch path too
                if (error.message && error.message.includes('Rate limit')) {
                    if (attempt < retries - 1) {
                        const delay = Math.min(2000 * Math.pow(2, attempt), 10000);
                        rpcQueue.rateLimitBackoff = Date.now() + delay;
                        await new Promise(r => setTimeout(r, delay));
                        continue;
                    }
                }
                if (attempt < retries - 1) {
                    await new Promise(r => setTimeout(r, 500 * (attempt + 1)));
                    continue;
                }
                if (!quiet) {
                    console.error(`RPC call ${method} failed:`, error.message);
                }
                throw error;
            }
        }
    });
}

/**
 * Fast-timeout wrapper for rpcCall - used for dashboard stats that need quick resolution.
 * Returns a promise that races the underlying rpcCall against a timeout.
 * @param {string} method
 * @param {any[]} params
 * @param {number} timeoutMs - Timeout in milliseconds (default 3000)
 */
function rpcCallFast(method, params = [], timeoutMs = 3000) {
    return Promise.race([
        rpcCall(method, params, { retries: 1, quiet: true }),
        new Promise((_, reject) => 
            setTimeout(() => reject(new Error(`fast timeout (${timeoutMs}ms)`)), timeoutMs)
        )
    ]);
}

/** Public RPC gateways often omit IronDAG `irondag_*` methods; use eth-only fallback after first failure. */
let mdsGetBlocksByStreamUnavailable = false;
let mdsGetStreamCountsUnavailable = false;

/**
 * Walk down from chain tip and collect up to maxEach blocks for stream B (single pass).
 */
async function sampleRecentStreamsBC(maxEach, latestBlock) {
    const blocksB = [];
    const maxScan = Math.min(4000, latestBlock + 1);
    for (let n = latestBlock, scanned = 0;
        blocksB.length < maxEach && scanned < maxScan;
        n--, scanned++) {
        const b = await rpcCall('eth_getBlockByNumber', [`0x${n.toString(16)}`, false], { retries: 2, quiet: true })
            .catch(() => null);
        if (!b) continue;
        if (getStreamType(b) === 'B') blocksB.push(b);
    }
    return { blocksB };
}

/**
 * Prefer irondag_getBlocksByStream; on missing method (typical behind /rpc proxies), scan with eth_getBlockByNumber.
 */
async function mdsGetBlocksByStreamCompat(streamLetter, count, latestBlock) {
    const want = Math.min(Math.max(1, count), 100);
    if (!mdsGetBlocksByStreamUnavailable) {
        try {
            const r = await rpcCall('irondag_getBlocksByStream', [streamLetter, want], { retries: 1, quiet: true });
            if (Array.isArray(r)) return r;
        } catch (e) {
            const msg = String(e && e.message ? e.message : e);
            if (msg.includes('Method not found')) {
                mdsGetBlocksByStreamUnavailable = true;
                if (!window.__mdsStreamFallbackLogged) {
                    console.info(
                        'Explorer: this RPC does not expose irondag_getBlocksByStream. Using eth_getBlockByNumber for stream filters.'
                    );
                    window.__mdsStreamFallbackLogged = true;
                }
            } else {
                throw e;
            }
        }
    }
    const out = [];
    const maxScan = Math.min(5000, latestBlock + 1);
    for (let n = latestBlock, scanned = 0; out.length < want && scanned < maxScan; n--, scanned++) {
        const b = await rpcCall('eth_getBlockByNumber', [`0x${n.toString(16)}`, false], { retries: 2, quiet: true })
            .catch(() => null);
        if (b && getStreamType(b) === streamLetter) out.push(b);
    }
    return out;
}

async function fetchDashboardStreamSamples(latestBlock) {
    if (!mdsGetBlocksByStreamUnavailable) {
        try {
            const blocksB = await rpcCall('irondag_getBlocksByStream', ['B', 100], { retries: 1, quiet: true });
            if (Array.isArray(blocksB)) {
                return { blocksB };
            }
        } catch (e) {
            const msg = String(e && e.message ? e.message : e);
            if (msg.includes('Method not found')) {
                mdsGetBlocksByStreamUnavailable = true;
                if (!window.__mdsStreamFallbackLogged) {
                    console.info(
                        'Explorer: this RPC does not expose irondag_getBlocksByStream. Using eth_getBlockByNumber for stream samples.'
                    );
                    window.__mdsStreamFallbackLogged = true;
                }
            } else {
                throw e;
            }
        }
    }
    return sampleRecentStreamsBC(100, latestBlock);
}

// ============================================================================
// DASHBOARD STATS
// ============================================================================

let lastBlockTimestamps = [];

// Block cache for TPS/block rate calculations
let blockCache = {
    blocks: [],          // Cached block data
    latestBlockNum: -1,  // Latest block number in cache
    maxBlocks: 20        // Maximum blocks to cache (keep low to avoid RPC flooding)
};

// DAG visualization globals (D3.js)
let dagSimulation = null; // D3 force simulation
let dagSvg = null; // D3 SVG selection
let dagZoom = null; // D3 zoom behavior
let dagNodes = []; // Current nodes array
let dagEdges = []; // Current edges array
let dagBlockCache = new Map(); // hash -> block data for DAG visualization
let latestBlockNumber = 0; // Track latest block for blue/red determination
let dagMaxBlocks = 30; // Maximum blocks to display in DAG
const DAG_REFRESH_INTERVAL = 15000; // 15 seconds between DAG updates

/**
 * Update DAG visualization incrementally - only add new blocks and remove old ones
 * Uses D3.js force simulation with gentle reheating on updates
 */
async function updateDagVisualization() {
    const container = document.getElementById('cytoscape-dag');
    if (!container || typeof d3 === 'undefined') {
        return;
    }

    // If D3 visualization doesn't exist, do full init
    if (!dagSvg) {
        return initDagVisualization();
    }

    const depthSelect = document.getElementById('dag-depth');
    const depth = parseInt(depthSelect?.value || '30');
    dagMaxBlocks = depth;

    try {
        // Fetch latest block number
        const latestHex = await rpcCall('eth_blockNumber', []);
        const latest = parseInt(latestHex, 16);
        
        // No new blocks? Skip update
        if (latest <= latestBlockNumber) {
            return;
        }

        const previousLatest = latestBlockNumber;
        latestBlockNumber = latest;

        // Calculate range: from previous latest to current latest, plus maintain window size
        const startBlock = Math.max(0, latest - depth + 1);
        const endBlock = latest;

        // Fetch only new blocks
        const newBlockPromises = [];
        for (let i = Math.max(startBlock, previousLatest + 1); i <= endBlock; i++) {
            newBlockPromises.push(rpcCall('eth_getBlockByNumber', ['0x' + i.toString(16), false]));
        }
        
        const newBlocks = await Promise.all(newBlockPromises);

        // Get current node hashes
        const currentNodeIds = new Set(dagNodes.map(n => n.id));
        const newBlockHashes = new Set();

        // Process new blocks into nodes
        const nodesToAdd = [];
        for (const block of newBlocks) {
            if (!block) continue;
            const hash = block.hash;
            newBlockHashes.add(hash);
            dagBlockCache.set(hash, block);

            const blockNum = parseInt(block.number, 16);
            const streamType = getStreamType(block);
            const isBlue = (latest - blockNum) >= 10;
            const timestamp = parseInt(block.timestamp, 16);
            const isRecent = (latest - blockNum) < 5; // Most recent 5 blocks pulse

            // Initial position based on block number (newer blocks on right)
            const xScale = container.clientWidth ? container.clientWidth * 0.8 : 800;
            const initialX = ((blockNum - startBlock) / depth) * xScale + 100;
            const initialY = (container.clientHeight || 500) / 2 + (Math.random() - 0.5) * 100;

            nodesToAdd.push({
                id: hash,
                blockNumber: blockNum,
                streamType: streamType,
                isBlue: isBlue,
                isRecent: isRecent,
                timestamp: timestamp,
                fullHash: hash,
                x: initialX,
                y: initialY,
                vx: 0,
                vy: 0
            });
        }

        // Add new edges
        const edgesToAdd = [];
        for (const block of newBlocks) {
            if (!block) continue;
            const parents = block.parentHashes || block.parent_hashes || (block.parentHash ? [block.parentHash] : []);
            for (const parentHash of parents) {
                if (parentHash && (currentNodeIds.has(parentHash) || newBlockHashes.has(parentHash))) {
                    edgesToAdd.push({
                        source: block.hash,
                        target: parentHash
                    });
                }
            }
        }

        // Update finalized status for existing nodes
        dagNodes.forEach(node => {
            node.isBlue = (latest - node.blockNumber) >= 10;
            node.isRecent = (latest - node.blockNumber) < 5;
        });

        // Remove old nodes (pruning)
        if (dagNodes.length + nodesToAdd.length > dagMaxBlocks) {
            // Sort by block number and remove oldest
            dagNodes.sort((a, b) => a.blockNumber - b.blockNumber);
            const numToRemove = dagNodes.length + nodesToAdd.length - dagMaxBlocks;
            const removedIds = new Set(dagNodes.slice(0, numToRemove).map(n => n.id));
            dagNodes = dagNodes.slice(numToRemove);
            // Also remove edges pointing to removed nodes
            dagEdges = dagEdges.filter(e => !removedIds.has(e.source.id || e.source) && !removedIds.has(e.target.id || e.target));
        }

        // Add new nodes and edges
        dagNodes = dagNodes.concat(nodesToAdd);
        dagEdges = dagEdges.concat(edgesToAdd);

        // Update D3 visualization
        renderDagNodes(latest);

        // Gently reheat the simulation (very mild to avoid jumps)
        if (dagSimulation && nodesToAdd.length > 0) {
            // Re-initialize simulation with updated nodes and edges
            dagSimulation.nodes(dagNodes);
            dagSimulation.force('link').links(dagEdges);
            dagSimulation.alpha(0.1).restart();
        }

    } catch (err) {
        console.error('DAG update error:', err);
        // On error, fall back to full reinit on next cycle
        dagSvg = null;
        dagSimulation = null;
    }
}

// Pagination state
const paginationState = {
    blocks: {
        page: 1,
        pageSize: 10,
        totalBlocks: 0
    },
    transactions: {
        page: 1,
        pageSize: 10,
        totalTx: 0
    }
};

// Stream filter state for blocks list
let activeStreamFilter = 'all';

// Address transaction history cache and state
const addressTxState = {
    cache: new Map(), // address -> { transactions: [], scannedToBlock: number, timestamp: number }
    currentPage: 1,
    pageSize: 10,
    maxBlocksToScan: 100,
    cacheTimeout: 60000 // 1 minute cache
};

/**
 * Navigate to previous page
 */
function prevPage(type) {
    if (paginationState[type].page > 1) {
        paginationState[type].page--;
        if (type === 'blocks') {
            loadRecentBlocks();
        } else {
            loadRecentTransactions();
        }
        updatePaginationUI(type);
    }
}

/**
 * Navigate to next page
 */
function nextPage(type) {
    paginationState[type].page++;
    if (type === 'blocks') {
        loadRecentBlocks();
    } else {
        loadRecentTransactions();
    }
    updatePaginationUI(type);
}

/**
 * Change page size
 */
function changePageSize(type, size) {
    paginationState[type].pageSize = parseInt(size);
    paginationState[type].page = 1; // Reset to first page
    if (type === 'blocks') {
        loadRecentBlocks();
    } else {
        loadRecentTransactions();
    }
    updatePaginationUI(type);
}

/**
 * Update pagination UI elements
 */
function updatePaginationUI(type) {
    const state = paginationState[type];
    const pageInfo = document.getElementById(`${type}-page-info`);
    const prevBtn = document.getElementById(`${type}-prev-btn`);
    const nextBtn = document.getElementById(`${type}-next-btn`);
    
    if (pageInfo) {
        const start = (state.page - 1) * state.pageSize + 1;
        const end = Math.min(state.page * state.pageSize, state.totalBlocks || state.totalTx || 0);
        pageInfo.textContent = `${start}-${end} of ${state.totalBlocks || state.totalTx || '--'}`;
    }
    
    if (prevBtn) {
        prevBtn.disabled = state.page <= 1;
        prevBtn.style.opacity = state.page <= 1 ? '0.5' : '1';
    }
    
    if (nextBtn) {
        // For blocks, check if we've reached the end
        const isLastPage = type === 'blocks' && (state.page * state.pageSize >= state.totalBlocks);
        nextBtn.disabled = isLastPage;
        nextBtn.style.opacity = isLastPage ? '0.5' : '1';
    }
}

/**
 * Fetch and cache blocks for TPS/block rate calculations
 * @param {number} latestBlock - The latest block number
 * @returns {Promise<Array>} - Array of cached blocks
 */
async function updateBlockCache(latestBlock) {
    const cacheSize = blockCache.maxBlocks;
    
    // If cache is empty or we have new blocks, update cache
    if (blockCache.blocks.length === 0 || blockCache.latestBlockNum !== latestBlock) {
        // Determine how many new blocks we need to fetch
        const blocksToFetch = Math.min(cacheSize, latestBlock + 1);
        
        // Fetch blocks in parallel
        const blockPromises = [];
        for (let i = 0; i < blocksToFetch; i++) {
            const blockNum = latestBlock - i;
            if (blockNum >= 0) {
                blockPromises.push(
                    rpcCall('eth_getBlockByNumber', [`0x${blockNum.toString(16)}`, false])
                        .then(block => ({
                            number: parseInt(block.number, 16),
                            timestamp: parseInt(block.timestamp, 16),
                            transactionCount: block.transactions ? (Array.isArray(block.transactions) ? block.transactions.length : parseInt(block.transactions, 16)) : 0
                        }))
                        .catch(() => null)
                );
            }
        }
        
        const fetchedBlocks = await Promise.all(blockPromises);
        blockCache.blocks = fetchedBlocks.filter(b => b !== null);
        blockCache.latestBlockNum = latestBlock;
    }
    
    return blockCache.blocks;
}

async function loadDashboard() {
    try {
        // Use Promise.allSettled for graceful degradation on individual call failures
        // Use rpcCallFast (3s timeout) for dashboard stats to avoid 10s hangs
        // Note: eth_blockNumber uses regular rpcCall (10s timeout) since it's critical and normally fast
        const results = await Promise.allSettled([
            rpcCall('eth_blockNumber'),
            rpcCallFast('irondag_getDagStats'),
            rpcCallFast('net_peerCount')
        ]);

        // Extract block number with caching to avoid flicker on timeout
        let blockNumber;
        if (results[0].status === 'fulfilled' && results[0].value) {
            const parsed = parseInt(results[0].value, 16) || 0;
            if (parsed > 0) {
                lastBlockNumber = results[0].value;
                blockNumber = results[0].value;
            } else if (lastBlockNumber) {
                blockNumber = lastBlockNumber;
                console.log('loadDashboard: using cached blockNumber (RPC returned 0)');
            } else {
                blockNumber = '0x0';
            }
        } else if (lastBlockNumber) {
            blockNumber = lastBlockNumber;
            console.log('loadDashboard: using cached blockNumber (RPC failed or timed out)');
        } else {
            blockNumber = '0x0';
        }
        
        // Use cached dagStats if call failed or returned null
        let dagStatsRaw = results[1].status === 'fulfilled' ? results[1].value : null;
        if (dagStatsRaw) {
            lastDagStats = dagStatsRaw;
        } else if (lastDagStats) {
            dagStatsRaw = lastDagStats;
            console.log('loadDashboard: using cached dagStats (RPC failed or timed out)');
        }
        
        let peerCountValue = null;
        if (results[2].status === 'fulfilled' && results[2].value) {
            peerCountValue = results[2].value;
            lastKnownPeerCount = peerCountValue;
            lastPeerCountUpdateTime = Date.now();
        } else if (lastKnownPeerCount) {
            // Keep showing last known peer count indefinitely — never drop to 0 on RPC timeout
            peerCountValue = lastKnownPeerCount;
            console.log('loadDashboard: using cached peerCount (RPC timeout, age=' + Math.round((Date.now() - lastPeerCountUpdateTime)/1000) + 's)');
        } else {
            peerCountValue = '0x0';
        }

        const dagStats = dagStatsRaw || { total_blocks: 0, total_transactions: 0, total_addresses: 0, blue_blocks: '0', red_blocks: '0' };
        const latestBlock = parseInt(blockNumber, 16) || 0;
        const totalTransactions = dagStats.total_transactions || 0;

        // Chain tip = eth_blockNumber (max header.block_number). Finalized = eth_getBlockByNumber("finalized") — not the same in a DAG.
        const tipEl = document.getElementById('chain-tip-block');
        if (tipEl) tipEl.textContent = formatNumber(latestBlock);

        let finalizedDisplay = lastFinalizedDisplay || '--';
        try {
            const fin = await rpcCall('eth_getBlockByNumber', ['finalized', false]);
            if (fin && fin.number != null) {
                const fn = parseInt(fin.number, 16);
                if (!Number.isNaN(fn)) {
                    finalizedDisplay = formatNumber(fn);
                    lastFinalizedDisplay = finalizedDisplay;
                }
            }
        } catch (_) {
            // Keep cached value if available, otherwise keep '--'
            if (lastFinalizedDisplay) {
                finalizedDisplay = lastFinalizedDisplay;
            }
        }
        const finEl = document.getElementById('last-finalized-block');
        if (finEl) finEl.textContent = finalizedDisplay;
        if (document.getElementById('total-transactions')) {
            document.getElementById('total-transactions').textContent = formatNumber(totalTransactions);
        }
        const peers = parseInt(peerCountValue, 16) || 0;
        if (document.getElementById('peer-count')) {
            document.getElementById('peer-count').textContent = formatNumber(peers);
        }

        // Update block cache and calculate stats from cached blocks
        const cachedBlocks = await updateBlockCache(latestBlock);
        
        // Calculate average block time and block rate from cached blocks
        calculateStatsFromCache(cachedBlocks);
        
        // Calculate TPS from cached blocks
        if (document.getElementById('tps') && cachedBlocks.length >= 2) {
            const tps = calculateTPSFromCache(cachedBlocks);
            document.getElementById('tps').textContent = tps;
        }
        
        // DAG Health card
        let healthPct;
        if (dagStatsRaw && (dagStatsRaw.blue_blocks !== undefined || dagStatsRaw.blueBlocks !== undefined)) {
            const blueBlocks = parseInt(dagStatsRaw.blue_blocks || dagStatsRaw.blueBlocks || '0', 16) || 0;
            const redBlocks = parseInt(dagStatsRaw.red_blocks || dagStatsRaw.redBlocks || '0', 16) || 0;
            const totalBlocks = blueBlocks + redBlocks;
            healthPct = totalBlocks > 0 ? Math.round((blueBlocks / totalBlocks) * 100) : 0;
            lastKnownHealth = healthPct;
            lastHealthUpdateTime = Date.now();
        } else if (lastKnownHealth !== null && (Date.now() - lastHealthUpdateTime) < HEALTH_STALE_TTL) {
            healthPct = lastKnownHealth;
        } else {
            healthPct = null;
        }
        const healthEl = document.getElementById('dag-health');
        if (healthEl) healthEl.textContent = healthPct !== null ? healthPct + '% Blue' : '--';
        const healthBar = document.getElementById('health-bar-blue');
        if (healthBar) healthBar.style.width = (healthPct !== null ? healthPct : 0) + '%';
        
        // BraidCore Distribution - try irondag_getStreamCounts first, then fallback to counting samples
        let streamA = 0, streamB = 0;
        let useExactCounts = false;

        // Try the new total-count method first
        if (!mdsGetStreamCountsUnavailable) {
            try {
                const counts = await rpcCall('irondag_getStreamCounts', [], { retries: 1, quiet: true });
                if (counts && typeof counts === 'object') {
                    streamA = counts.A || 0;
                    streamB = counts.B || 0;
                    useExactCounts = true;
                }
            } catch (e) {
                const msg = String(e && e.message ? e.message : e);
                if (msg.includes('not found') || msg.includes('not available')) {
                    mdsGetStreamCountsUnavailable = true;
                    if (!window.__mdsStreamCountsFallbackLogged) {
                        console.info('Explorer: irondag_getStreamCounts not available, using fallback method.');
                        window.__mdsStreamCountsFallbackLogged = true;
                    }
                }
            }
        }

        // Fallback: count Stream A from cache, B from RPC samples
        if (!useExactCounts) {
            for (const [hash, block] of dagBlockCache) {
                const st = getStreamType(block);
                if (st === 'A') streamA++;
            }
            // Stream B: irondag_getBlocksByStream if present; else eth scan (public /rpc proxies often omit irondag_*).
            try {
                const { blocksB } = await fetchDashboardStreamSamples(latestBlock);
                // Show "100+" when we hit the limit, indicating there may be more
                if (Array.isArray(blocksB) && blocksB.length >= 100) {
                    streamB = '100+';
                } else {
                    streamB = Array.isArray(blocksB) ? blocksB.length : 0;
                }
            } catch (e) {
                console.error('Error fetching stream B counts:', e);
            }
        }

        // Calculate total for percentage (use numeric values)
        const totalA = typeof streamA === 'number' ? streamA : 0;
        const totalB = typeof streamB === 'number' ? streamB : 100;
        const streamTotal = totalA + totalB;

        const distEl = document.getElementById('stream-dist');
        if (distEl && streamTotal > 0) {
            distEl.textContent = `A:${streamA} B:${streamB}`;
        }
        if (streamTotal > 0) {
            const segA = document.getElementById('stream-seg-a');
            const segB = document.getElementById('stream-seg-b');
            if (segA) segA.style.width = (totalA/streamTotal*100) + '%';
            if (segB) segB.style.width = (totalB/streamTotal*100) + '%';
        }

    } catch (error) {
        console.error('Error loading dashboard:', error);
    }
}

/**
 * Calculate TPS from cached blocks
 * TPS = total transactions in window / time span in seconds
 */
function calculateTPSFromCache(blocks) {
    if (blocks.length < 2) return '--';
    
    // Sort blocks by number (descending to ascending for time calculation)
    const sortedBlocks = [...blocks].sort((a, b) => a.number - b.number);
    
    // Sum all transactions in the window
    const totalTx = sortedBlocks.reduce((sum, block) => sum + block.transactionCount, 0);
    
    // Calculate time span from oldest to newest block
    const oldestTimestamp = sortedBlocks[0].timestamp;
    const newestTimestamp = sortedBlocks[sortedBlocks.length - 1].timestamp;
    const timeSpanSeconds = newestTimestamp - oldestTimestamp;
    
    if (timeSpanSeconds <= 0) return '--';
    
    const tps = totalTx / timeSpanSeconds;
    return tps.toFixed(1);
}

/**
 * Calculate block time and rate from cached blocks
 */
function calculateStatsFromCache(blocks) {
    if (blocks.length < 2) {
        if (document.getElementById('avg-block-time')) {
            document.getElementById('avg-block-time').textContent = '--';
        }
        if (document.getElementById('block-rate')) {
            document.getElementById('block-rate').textContent = '--';
        }
        return;
    }
    
    // Sort blocks by number (descending)
    const sortedBlocks = [...blocks].sort((a, b) => b.number - a.number);
    
    // Collect valid time diffs for median calculation
    const timeDiffs = [];
    
    for (let i = 0; i < sortedBlocks.length - 1; i++) {
        const t1 = sortedBlocks[i].timestamp;
        const t2 = sortedBlocks[i + 1].timestamp;
        const diff = Math.abs(t1 - t2);
        if (diff > 0 && diff < 3600) { // Sanity check: ignore gaps > 1 hour
            timeDiffs.push(diff);
        }
    }
    
    if (timeDiffs.length === 0) {
        if (document.getElementById('avg-block-time')) {
            document.getElementById('avg-block-time').textContent = '--';
        }
        if (document.getElementById('block-rate')) {
            document.getElementById('block-rate').textContent = '--';
        }
        return;
    }
    
    // Use median instead of mean to ignore outlier gaps from downtime
    timeDiffs.sort((a, b) => a - b);
    const medianTime = timeDiffs[Math.floor(timeDiffs.length / 2)];
    
    if (document.getElementById('avg-block-time')) {
        document.getElementById('avg-block-time').textContent = `${medianTime.toFixed(1)}`;
    }
    
    // Calculate blocks per second
    if (document.getElementById('block-rate')) {
        const blockRate = medianTime > 0 ? (1 / medianTime).toFixed(2) : '0.00';
        document.getElementById('block-rate').textContent = blockRate;
    }
    
    console.log(`Stats from ${blocks.length} blocks: median time ${medianTime.toFixed(2)}s, block rate ${(1/medianTime).toFixed(2)}/s`);
}

// ============================================================================
// DAG VISUALIZATION (Cytoscape.js)
// ============================================================================

/**
 * Get stream type from block data
 * @param {Object} block - Block object
 * @returns {string} - 'A', 'B', or 'C'
 */
function getStreamType(block) {
    // Check various field names the RPC might use
    const st = block.streamType || block.stream_type;
    if (st !== undefined && st !== null) {
        // Check string labels first (RPC returns "A", "B", "C")
        const upper = String(st).toUpperCase();
        if (upper === 'A' || upper === 'STREAMA') return 'A';
        if (upper === 'B' || upper === 'STREAMB') return 'B';
        // Numeric fallback (0=A, 1=B)
        const num = typeof st === 'string' ? parseInt(st, 16) : st;
        if (num === 0) return 'A';
        if (num === 1) return 'B';
    }
    // Fallback: default to A
    return 'A';
}

/**
 * Initialize the main DAG visualization with D3.js force simulation
 */
async function initDagVisualization() {
    const container = document.getElementById('cytoscape-dag');
    if (!container || typeof d3 === 'undefined') {
        console.log('D3 container not found or d3 not loaded');
        return;
    }

    const depthSelect = document.getElementById('dag-depth');
    const depth = parseInt(depthSelect?.value || '30');
    dagMaxBlocks = depth;

    try {
        // Show loading state
        container.innerHTML = '<div class="dag-loading"><i class="fas fa-spinner fa-spin"></i> Loading DAG...</div>';

        // Fetch latest block number
        const latestHex = await rpcCall('eth_blockNumber', []);
        const latest = parseInt(latestHex, 16);
        latestBlockNumber = latest;
        const startBlock = Math.max(0, latest - depth + 1);

        // Fetch blocks
        const blockPromises = [];
        for (let i = startBlock; i <= latest; i++) {
            blockPromises.push(rpcCall('eth_getBlockByNumber', ['0x' + i.toString(16), false]));
        }
        const blocks = await Promise.all(blockPromises);

        // Filter out null blocks
        const validBlocks = blocks.filter(b => b != null);
        if (validBlocks.length === 0) {
            container.innerHTML = '<div class="dag-error"><i class="fas fa-exclamation-triangle"></i><div>No blocks available for visualization</div></div>';
            return;
        }

        // Track all block hashes
        const knownHashes = new Set();

        // Build nodes and edges arrays
        dagNodes = [];
        dagEdges = [];

        for (const block of validBlocks) {
            const hash = block.hash;
            knownHashes.add(hash);
            dagBlockCache.set(hash, block);

            const blockNum = parseInt(block.number, 16);
            const streamType = getStreamType(block);
            const isBlue = (latest - blockNum) >= 10;
            const isRecent = (latest - blockNum) < 5;
            const timestamp = parseInt(block.timestamp, 16);

            // Initial position (will be adjusted by force simulation)
            dagNodes.push({
                id: hash,
                blockNumber: blockNum,
                streamType: streamType,
                isBlue: isBlue,
                isRecent: isRecent,
                timestamp: timestamp,
                fullHash: hash,
                x: 100 + (blockNum - startBlock) * 80,
                y: 250 + (Math.random() - 0.5) * 100
            });
        }

        // Build edges
        for (const block of validBlocks) {
            const parents = block.parentHashes || block.parent_hashes || (block.parentHash ? [block.parentHash] : []);
            for (const parentHash of parents) {
                if (parentHash && knownHashes.has(parentHash)) {
                    dagEdges.push({
                        source: block.hash,
                        target: parentHash
                    });
                }
            }
        }

        // Clear container and create SVG
        container.innerHTML = '';
        
        // Ensure container has dimensions
        let width = container.clientWidth || 800;
        let height = container.clientHeight || 500;
        
        // Minimum dimensions to ensure visibility
        width = Math.max(width, 400);
        height = Math.max(height, 300);

        // Create SVG with zoom behavior
        dagSvg = d3.select(container)
            .append('svg')
            .attr('width', '100%')
            .attr('height', '100%')
            .attr('viewBox', `0 0 ${width} ${height}`);

        // Create defs for gradients, filters, and markers
        const defs = dagSvg.append('defs');

        // Radial gradients for each stream type
        const streamColors = {
            A: { inner: '#60a5fa', outer: '#3b82f6' },
            B: { inner: '#a78bfa', outer: '#8b5cf6' }
        };

        Object.entries(streamColors).forEach(([stream, colors]) => {
            const gradient = defs.append('radialGradient')
                .attr('id', `dag-gradient-${stream}`)
                .attr('cx', '30%')
                .attr('cy', '30%')
                .attr('r', '70%');
            gradient.append('stop').attr('offset', '0%').attr('stop-color', colors.inner);
            gradient.append('stop').attr('offset', '100%').attr('stop-color', colors.outer);
        });

        // Glow filter
        const glowFilter = defs.append('filter')
            .attr('id', 'dag-glow')
            .attr('x', '-50%')
            .attr('y', '-50%')
            .attr('width', '200%')
            .attr('height', '200%');
        glowFilter.append('feGaussianBlur').attr('stdDeviation', '3').attr('result', 'blur');
        const glowMerge = glowFilter.append('feMerge');
        glowMerge.append('feMergeNode').attr('in', 'blur');
        glowMerge.append('feMergeNode').attr('in', 'SourceGraphic');

        // Bright glow filter for hover
        const brightGlowFilter = defs.append('filter')
            .attr('id', 'dag-glow-bright')
            .attr('x', '-50%')
            .attr('y', '-50%')
            .attr('width', '200%')
            .attr('height', '200%');
        brightGlowFilter.append('feGaussianBlur').attr('stdDeviation', '5').attr('result', 'blur');
        const brightGlowMerge = brightGlowFilter.append('feMerge');
        brightGlowMerge.append('feMergeNode').attr('in', 'blur');
        brightGlowMerge.append('feMergeNode').attr('in', 'SourceGraphic');

        // Drop shadow filter
        const shadowFilter = defs.append('filter')
            .attr('id', 'dag-shadow')
            .attr('x', '-20%')
            .attr('y', '-20%')
            .attr('width', '140%')
            .attr('height', '140%');
        shadowFilter.append('feDropShadow')
            .attr('dx', '2')
            .attr('dy', '2')
            .attr('stdDeviation', '3')
            .attr('flood-color', 'rgba(0,0,0,0.5)');

        // Arrow marker
        defs.append('marker')
            .attr('id', 'dag-arrow')
            .attr('viewBox', '0 -5 10 10')
            .attr('refX', 20)
            .attr('refY', 0)
            .attr('markerWidth', 6)
            .attr('markerHeight', 6)
            .attr('orient', 'auto')
            .append('path')
            .attr('d', 'M0,-5L10,0L0,5')
            .attr('class', 'dag-arrow');

        // Create zoom behavior
        dagZoom = d3.zoom()
            .scaleExtent([0.1, 4])
            .on('zoom', (event) => {
                mainGroup.attr('transform', event.transform);
            });

        // Create main group for zoom/pan
        const mainGroup = dagSvg.append('g').attr('class', 'dag-main-group');
        dagSvg.call(dagZoom);

        // Create edge group (rendered first, so edges are behind nodes)
        const edgeGroup = mainGroup.append('g').attr('class', 'dag-edge-group');

        // Create node group
        const nodeGroup = mainGroup.append('g').attr('class', 'dag-node-group');

        // Create force simulation
        dagSimulation = d3.forceSimulation(dagNodes)
            .force('link', d3.forceLink(dagEdges)
                .id(d => d.id)
                .distance(80)
                .strength(0.5))
            .force('charge', d3.forceManyBody().strength(-150))
            .force('x', d3.forceX()
                .x(d => {
                    // Position by block number using CURRENT globals (not stale closure vars)
                    const allNums = dagNodes.map(n => n.blockNumber);
                    const minBlock = Math.min(...allNums);
                    const maxBlock = Math.max(...allNums);
                    const range = maxBlock - minBlock || 1;
                    const xPos = ((d.blockNumber - minBlock) / range) * (width * 0.7) + width * 0.15;
                    return xPos;
                })
                .strength(0.3))
            .force('y', d3.forceY(height / 2).strength(0.1))
            .force('collision', d3.forceCollide().radius(45))
            .alphaDecay(0.05)
            .on('tick', () => {
                // Update edge paths
                edgeGroup.selectAll('path').attr('d', d => {
                    // Handle unresolved edges (source/target still strings) or missing coordinates
                    const source = typeof d.source === 'object' ? d.source : dagNodes.find(n => n.id === d.source);
                    const target = typeof d.target === 'object' ? d.target : dagNodes.find(n => n.id === d.target);
                    if (!source || !target || source.x == null || target.x == null) {
                        return ''; // Skip edges that aren't ready
                    }
                    const sourceX = source.x;
                    const sourceY = source.y;
                    const targetX = target.x;
                    const targetY = target.y;
                    // Curved path
                    const dx = targetX - sourceX;
                    const dy = targetY - sourceY;
                    const dr = Math.sqrt(dx * dx + dy * dy) * 0.8;
                    return `M${sourceX},${sourceY}A${dr},${dr} 0 0,1 ${targetX},${targetY}`;
                });

                // Update node positions
                nodeGroup.selectAll('.dag-node-group')
                    .attr('transform', d => `translate(${d.x || 0},${d.y || 0})`);
            });

        // Initial render
        renderDagNodes(latest);

        // Wire up controls
        document.getElementById('dag-fit')?.addEventListener('click', () => {
            dagSvg.transition().duration(500).call(dagZoom.transform, d3.zoomIdentity);
        });
        document.getElementById('dag-zoom-in')?.addEventListener('click', () => {
            dagSvg.transition().duration(300).call(dagZoom.scaleBy, 1.3);
        });
        document.getElementById('dag-zoom-out')?.addEventListener('click', () => {
            dagSvg.transition().duration(300).call(dagZoom.scaleBy, 0.7);
        });
        document.getElementById('dag-depth')?.addEventListener('change', () => {
            // When depth changes, we need a full reinit
            dagSvg = null;
            dagSimulation = null;
            dagNodes = [];
            dagEdges = [];
            initDagVisualization();
        });

        // Initial fit
        setTimeout(() => {
            const bounds = mainGroup.node().getBBox();
            if (bounds.width > 0 && bounds.height > 0) {
                const scale = Math.min(
                    width / (bounds.width + 60),
                    height / (bounds.height + 60),
                    1.5
                );
                const translateX = width / 2 - (bounds.x + bounds.width / 2) * scale;
                const translateY = height / 2 - (bounds.y + bounds.height / 2) * scale;
                dagSvg.transition().duration(500).call(
                    dagZoom.transform,
                    d3.zoomIdentity.translate(translateX, translateY).scale(scale)
                );
            }
        }, 500);

    } catch (err) {
        console.error('DAG visualization error:', err);
        container.innerHTML = '<div class="dag-error"><i class="fas fa-exclamation-triangle"></i><div>DAG visualization unavailable</div></div>';
    }
}

/**
 * Render/update DAG nodes using D3
 * @param {number} latest - Latest block number for status determination
 */
function renderDagNodes(latest) {
    if (!dagSvg) return;

    const mainGroup = dagSvg.select('.dag-main-group');
    const edgeGroup = mainGroup.select('.dag-edge-group');
    const nodeGroup = mainGroup.select('.dag-node-group');

    // Update edges
    const edgePaths = edgeGroup.selectAll('path')
        .data(dagEdges, d => `${d.source.id || d.source}-${d.target.id || d.target}`);

    edgePaths.exit().remove();

    edgePaths.enter()
        .append('path')
        .attr('class', 'dag-edge-path')
        .attr('stroke', '#6366f1')
        .attr('stroke-width', 1.5)
        .attr('stroke-opacity', 0.6)
        .attr('marker-end', 'url(#dag-arrow)');

    // Update nodes
    const nodes = nodeGroup.selectAll('.dag-node-group')
        .data(dagNodes, d => d.id);

    // Remove exiting nodes with fade-out animation
    nodes.exit()
        .classed('exiting', true)
        .transition()
        .duration(300)
        .style('opacity', 0)
        .remove();

    // Create new node groups
    const nodeEnter = nodes.enter()
        .append('g')
        .attr('class', d => `dag-node-group ${d.isRecent ? 'pending-block' : ''}`)
        .classed('entering', true)
        .on('click', (event, d) => {
            event.stopPropagation();
            showBlockDetail(d.id);
        })
        .on('mouseenter', (event, d) => {
            showDagTooltipD3(d, event);
        })
        .on('mouseleave', () => {
            hideDagTooltip();
        });

    // Add pulse ring for recent blocks
    nodeEnter.filter(d => d.isRecent)
        .append('circle')
        .attr('class', 'dag-pulse-ring')
        .attr('r', 40);

    // Add status ring (finalized/pending indicator)
    nodeEnter.append('circle')
        .attr('class', d => `dag-status-ring ${d.isBlue ? 'finalized' : 'pending'}`)
        .attr('r', 38);

    // Add main node rectangle with rounded corners
    nodeEnter.append('rect')
        .attr('class', 'dag-node-rect')
        .attr('x', -35)
        .attr('y', -18)
        .attr('width', 70)
        .attr('height', 36)
        .attr('rx', 6)
        .attr('ry', 6)
        .attr('fill', d => `url(#dag-gradient-${d.streamType})`)
        .attr('stroke', d => d.isBlue ? '#10b981' : '#f59e0b')
        .attr('stroke-width', 2.5)
        .attr('filter', 'url(#dag-glow)');

    // Add block number text
    nodeEnter.append('text')
        .attr('class', 'dag-node-text')
        .attr('y', -3)
        .text(d => `#${d.blockNumber}`);

    // Add hash text
    nodeEnter.append('text')
        .attr('class', 'dag-node-hash')
        .attr('y', 10)
        .text(d => d.id.slice(0, 8));

    // Merge enter and update selections
    nodeEnter.merge(nodes)
        .attr('class', d => `dag-node-group ${d.isRecent ? 'pending-block' : ''}`)
        .select('.dag-status-ring')
        .attr('class', d => `dag-status-ring ${d.isBlue ? 'finalized' : 'pending'}`);

    // Update simulation nodes/links if simulation exists
    if (dagSimulation) {
        dagSimulation.nodes(dagNodes);
        dagSimulation.force('link').links(dagEdges);
    }
}

/**
 * Highlight a specific block in the main DAG view
 * @param {string} hash - Block hash to highlight
 */
function highlightInDag(hash) {
    if (!dagSvg) return;
    
    // Scroll to DAG section
    document.querySelector('.dag-explorer-section')?.scrollIntoView({ behavior: 'smooth' });
    
    // Remove previous highlights
    dagSvg.selectAll('.dag-node-group').classed('highlighted', false);
    
    // Find and highlight the node
    const node = dagNodes.find(n => n.id === hash);
    if (node) {
        dagSvg.select('.dag-main-group').selectAll('.dag-node-group')
            .filter(d => d.id === hash)
            .classed('highlighted', true)
            .select('.dag-node-rect')
            .transition()
            .duration(300)
            .attr('stroke', '#fbbf24')
            .attr('stroke-width', 4);
        
        // Zoom to node
        if (node.x !== undefined && node.y !== undefined) {
            dagSvg.transition().duration(500).call(
                dagZoom.transform,
                d3.zoomIdentity
                    .translate(dagSvg.node().clientWidth / 2, dagSvg.node().clientHeight / 2)
                    .scale(1.5)
                    .translate(-node.x, -node.y)
            );
        }
    }
}

/**
 * Show tooltip for a DAG node on hover (D3 version)
 * @param {Object} d - D3 node data object
 * @param {Event} event - Original mouse event for positioning
 */
function showDagTooltipD3(d, event) {
    const tooltip = document.getElementById('dag-tooltip');
    if (!tooltip) return;
    
    const hash = d.fullHash || d.id;
    const blockNum = d.blockNumber;
    const streamType = d.streamType || 'A';
    const isBlue = d.isBlue;
    const timestamp = d.timestamp;
    
    // Format timestamp if available
    let timeStr = 'N/A';
    if (timestamp) {
        const date = new Date(timestamp * 1000);
        timeStr = date.toLocaleString();
    }
    
    // Determine status
    const status = isBlue ? 'Finalized' : 'Pending';
    const statusClass = isBlue ? 'finalized' : 'pending';
    
    // Build tooltip content
    tooltip.innerHTML = `
        <div class="dag-tooltip-header">
            <i class="fas fa-cube"></i>
            Block #${blockNum}
        </div>
        <div class="dag-tooltip-row">
            <span class="dag-tooltip-label">Hash</span>
            <span class="dag-tooltip-value" style="font-size: 0.7rem;">${hash.slice(0, 16)}...</span>
        </div>
        <div class="dag-tooltip-row">
            <span class="dag-tooltip-label">Stream</span>
            <span class="dag-tooltip-value stream-${streamType.toLowerCase()}">Stream ${streamType}</span>
        </div>
        <div class="dag-tooltip-row">
            <span class="dag-tooltip-label">Status</span>
            <span class="dag-tooltip-status ${statusClass}">${status}</span>
        </div>
        <div class="dag-tooltip-row">
            <span class="dag-tooltip-label">Timestamp</span>
            <span class="dag-tooltip-value">${timeStr}</span>
        </div>
    `;
    
    // Position tooltip near cursor
    const padding = 15;
    let left = event.clientX + padding;
    let top = event.clientY + padding;
    
    // Adjust if tooltip would go off-screen
    tooltip.classList.remove('hidden');
    const rect = tooltip.getBoundingClientRect();
    
    if (left + rect.width > window.innerWidth) {
        left = event.clientX - rect.width - padding;
    }
    if (top + rect.height > window.innerHeight) {
        top = event.clientY - rect.height - padding;
    }
    
    tooltip.style.left = left + 'px';
    tooltip.style.top = top + 'px';
}

/**
 * Hide the DAG tooltip
 */
function hideDagTooltip() {
    const tooltip = document.getElementById('dag-tooltip');
    if (tooltip) {
        tooltip.classList.add('hidden');
    }
}

// ============================================================================
// BLOCKS LIST
// ============================================================================

async function loadRecentBlocks() {
    const blocksList = document.getElementById('blocks-list');
    
    try {
        const blockNumberHex = await rpcCall('eth_blockNumber');
        const latestBlock = parseInt(blockNumberHex, 16);
        
        // Update total blocks count for pagination
        paginationState.blocks.totalBlocks = latestBlock + 1;
        
        if (latestBlock === 0) {
            blocksList.innerHTML = '<div class="loading-placeholder">No blocks found</div>';
            return;
        }
        
        const { page, pageSize } = paginationState.blocks;
        
        // Use stream-specific RPC when available; otherwise scan eth_getBlockByNumber from tip.
        if (activeStreamFilter !== 'all') {
            const count = pageSize; // For pagination, we could use page * pageSize and slice
            const result = await mdsGetBlocksByStreamCompat(activeStreamFilter, count, latestBlock);
            
            if (result && Array.isArray(result)) {
                const displayBlocks = result;
                
                if (displayBlocks.length === 0) {
                    blocksList.innerHTML = `<div class="loading-placeholder">No blocks found for Stream ${activeStreamFilter}</div>`;
                    return;
                }
                
                blocksList.innerHTML = displayBlocks.map(block => {
                    const blockNum = parseInt(block.number, 16);
                    const timestamp = parseInt(block.timestamp, 16);
                    const txCount = block.transactions ? block.transactions.length : 0;
                    const miner = block.miner || '0x0000000000000000000000000000000000000000';
                    
                    // Stream type badge
                    const streamType = getStreamType(block);
                    const streamBadge = `<span class="badge badge-stream-${streamType.toLowerCase()}">${streamType}</span>`;
                    
                    // Consensus indicator (blue/red)
                    const isBlue = (latestBlock - blockNum) >= 10;
                    const consensusDot = isBlue 
                        ? '<span class="consensus-dot consensus-blue" title="Finalized"></span>' 
                        : '<span class="consensus-dot consensus-red" title="Pending"></span>';
                    
                    // Parent count if multiple parents
                    const parents = block.parentHashes || block.parent_hashes || [];
                    const parentInfo = parents.length > 1 
                        ? `<span class="parent-count">${parents.length} parents</span>` 
                        : '';
                    
                    return `
                        <div class="block-item">
                            <div class="block-icon">
                                <i class="fas fa-cube"></i>
                            </div>
                            <div class="block-info">
                                <span class="block-number">${consensusDot}<span onclick="showBlockDetail('${block.hash}')">${blockNum}</span> ${streamBadge}</span>
                                <div class="block-meta">
                                    <span>Txn: ${txCount}</span>
                                    <span>Reward: 0</span>
                                    <span class="miner-address" onclick="showAddressDetail('${miner}')" title="${miner}">
                                        ${truncateAddress(miner)}
                                    </span>
                                    ${parentInfo}
                                </div>
                            </div>
                            <div class="block-time" data-timestamp="${timestamp}">${timeAgo(timestamp)}</div>
                        </div>
                    `;
                }).join('');
                
                // Update pagination UI
                updatePaginationUI('blocks');
            }
            return;
        }
        
        // Original logic for 'all' streams
        // When filtering by stream, fetch more blocks to account for filtering
        const fetchMultiplier = activeStreamFilter === 'all' ? 1 : 3;
        const blocksToFetch = pageSize * fetchMultiplier;
        const startBlock = latestBlock - ((page - 1) * blocksToFetch);
        const blockPromises = [];
        
        for (let i = 0; i < blocksToFetch && (startBlock - i) >= 0; i++) {
            const blockNum = startBlock - i;
            blockPromises.push(
                rpcCall('eth_getBlockByNumber', [`0x${blockNum.toString(16)}`, true])
                    .catch(() => null)
            );
        }
        
        const blocks = await Promise.all(blockPromises);
        let validBlocks = blocks.filter(b => b !== null);
        
        // Filter by active stream
        let displayBlocks = validBlocks;
        if (activeStreamFilter !== 'all') {
            displayBlocks = validBlocks.filter(block => getStreamType(block) === activeStreamFilter);
        }
        
        // Limit to pageSize for display
        displayBlocks = displayBlocks.slice(0, pageSize);
        
        if (displayBlocks.length === 0) {
            const filterMsg = activeStreamFilter === 'all' ? '' : ` for Stream ${activeStreamFilter}`;
            blocksList.innerHTML = `<div class="loading-placeholder">No blocks found${filterMsg}</div>`;
            return;
        }
        
        blocksList.innerHTML = displayBlocks.map(block => {
            const blockNum = parseInt(block.number, 16);
            const timestamp = parseInt(block.timestamp, 16);
            const txCount = block.transactions ? block.transactions.length : 0;
            const miner = block.miner || '0x0000000000000000000000000000000000000000';
            
            // Stream type badge
            const streamType = getStreamType(block);
            const streamBadge = `<span class="badge badge-stream-${streamType.toLowerCase()}">${streamType}</span>`;
            
            // Consensus indicator (blue/red)
            const isBlue = (latestBlock - blockNum) >= 10;
            const consensusDot = isBlue 
                ? '<span class="consensus-dot consensus-blue" title="Finalized"></span>' 
                : '<span class="consensus-dot consensus-red" title="Pending"></span>';
            
            // Parent count if multiple parents
            const parents = block.parentHashes || block.parent_hashes || [];
            const parentInfo = parents.length > 1 
                ? `<span class="parent-count">${parents.length} parents</span>` 
                : '';
            
            return `
                <div class="block-item">
                    <div class="block-icon">
                        <i class="fas fa-cube"></i>
                    </div>
                    <div class="block-info">
                        <span class="block-number">${consensusDot}<span onclick="showBlockDetail('${block.hash}')">${blockNum}</span> ${streamBadge}</span>
                        <div class="block-meta">
                            <span>Txn: ${txCount}</span>
                            <span>Reward: 0</span>
                            <span class="miner-address" onclick="showAddressDetail('${miner}')" title="${miner}">
                                ${truncateAddress(miner)}
                            </span>
                            ${parentInfo}
                        </div>
                    </div>
                    <div class="block-time" data-timestamp="${timestamp}">${timeAgo(timestamp)}</div>
                </div>
            `;
        }).join('');
        
        // Update pagination UI
        updatePaginationUI('blocks');
        
    } catch (error) {
        console.error('Error loading blocks:', error);
        blocksList.innerHTML = '<div class="loading-placeholder text-error">Error loading blocks</div>';
    }
}

// ============================================================================
// TRANSACTIONS LIST
// ============================================================================

async function loadRecentTransactions() {
    const txList = document.getElementById('transactions-list');
    
    try {
        const blockNumberHex = await rpcCall('eth_blockNumber');
        const latestBlock = parseInt(blockNumberHex, 16);
        
        if (latestBlock === 0) {
            txList.innerHTML = '<div class="loading-placeholder">No transactions found</div>';
            paginationState.transactions.totalTx = 0;
            updatePaginationUI('transactions');
            return;
        }
        
        const { page, pageSize } = paginationState.transactions;
        const transactions = [];
        const skipCount = (page - 1) * pageSize;
        let blocksChecked = 0;
        let currentBlock = latestBlock;
        
        // Fetch blocks until we have enough transactions for current page
        let consecutiveEmpty = 0;
        while (transactions.length < pageSize && currentBlock >= 0) {
            try {
                const block = await rpcCall('eth_getBlockByNumber', [`0x${currentBlock.toString(16)}`, true]);

                if (block && block.transactions && block.transactions.length > 0) {
                    consecutiveEmpty = 0;
                    for (const tx of block.transactions) {
                        // Skip transactions from previous pages
                        if (skipCount > 0 && blocksChecked === 0) {
                            // This is the first block we're checking, skip older transactions
                            continue;
                        }
                        if (transactions.length >= pageSize) break;
                        transactions.push({
                            ...tx,
                            blockNumber: currentBlock,
                            blockTimestamp: parseInt(block.timestamp, 16)
                        });
                    }
                } else {
                    consecutiveEmpty++;
                }

                blocksChecked++;
                currentBlock--;

                // Stop early if we've scanned many blocks and found nothing — mining-only chain
                if (consecutiveEmpty >= 50 && transactions.length === 0) break;
                // Hard cap on total blocks scanned
                if (blocksChecked > 1000) break;

            } catch (error) {
                console.error(`Error loading block ${currentBlock}:`, error);
                currentBlock--;
            }
        }
        
        // For transactions pagination, we estimate total based on blocks scanned
        // This is an approximation since we can't easily count all transactions
        paginationState.transactions.totalTx = (page * pageSize) + (transactions.length < pageSize ? 0 : pageSize);
        
        if (transactions.length === 0) {
            txList.innerHTML = '<div class="loading-placeholder">No transactions found</div>';
            updatePaginationUI('transactions');
            return;
        }
        
        // Fetch transaction receipts to get actual status
        const txStatuses = await Promise.all(
            transactions.map(tx => getTransactionStatus(tx.hash))
        );
        
        txList.innerHTML = transactions.map((tx, index) => {
            const txType = getTransactionType(tx);
            const value = parseInt(tx.value || '0x0', 16) / 1e18;
            const gasPrice = parseInt(tx.gasPrice || '0x0', 16);
            const gas = parseInt(tx.gas || '0x0', 16);
            const fee = (gasPrice * gas) / 1e18;
            const status = txStatuses[index];
            
            return `
                <div class="tx-item">
                    <div class="tx-header">
                        <div class="tx-badges">
                            ${getTxTypeBadge(txType)}
                            ${getTxStatusBadge(status)}
                        </div>
                    </div>
                    <div class="tx-body">
                        <div class="tx-addresses">
                            <div class="tx-address-row">
                                <div class="address-avatar from">F</div>
                                <span class="address-text" onclick="showAddressDetail('${tx.from}')" title="${tx.from}">
                                    ${truncateAddress(tx.from)}
                                </span>
                                <button class="copy-btn" onclick="copyToClipboard('${tx.from}')">
                                    <i class="fas fa-copy"></i>
                                </button>
                            </div>
                            <div class="tx-address-row">
                                <div class="address-avatar to">T</div>
                                <span class="address-text" onclick="showAddressDetail('${tx.to || 'Contract'}')" title="${tx.to || 'Contract Creation'}">
                                    ${tx.to ? truncateAddress(tx.to) : 'Contract Creation'}
                                </span>
                                ${tx.to ? `<button class="copy-btn" onclick="copyToClipboard('${tx.to}')"><i class="fas fa-copy"></i></button>` : ''}
                            </div>
                        </div>
                        <div class="tx-details">
                            <span class="tx-hash" onclick="showTxDetail('${tx.hash}')" title="${tx.hash}">
                                ${truncateHash(tx.hash)}
                            </span>
                            <span class="tx-time" data-timestamp="${tx.blockTimestamp}">${timeAgo(tx.blockTimestamp)}</span>
                            <span class="tx-value">Value ${value.toFixed(4)} IDAG</span>
                            <span class="tx-fee">Fee ${fee.toFixed(6)} IDAG</span>
                        </div>
                    </div>
                </div>
            `;
        }).join('');
        
        // Update pagination UI
        updatePaginationUI('transactions');
        
    } catch (error) {
        console.error('Error loading transactions:', error);
        txList.innerHTML = '<div class="loading-placeholder text-error">Error loading transactions</div>';
    }
}

// ============================================================================
// SEARCH FUNCTIONALITY
// ============================================================================

function setupSearch() {
    const searchInput = document.getElementById('search-input');
    const searchBtn = document.getElementById('search-btn');
    
    const performSearch = async () => {
        const query = searchInput.value.trim();
        if (!query) return;
        
        try {
            // Determine search type
            if (query.startsWith('0x') && query.length === 66) {
                // Block hash or transaction hash
                try {
                    const block = await rpcCall('eth_getBlockByHash', [query, true]);
                    if (block) {
                        showBlockDetail(query);
                        return;
                    }
                } catch (e) {}
                
                try {
                    const tx = await rpcCall('eth_getTransactionByHash', [query]);
                    if (tx) {
                        showTxDetail(query);
                        return;
                    }
                } catch (e) {}
                
            } else if (query.startsWith('0x') && query.length === 42) {
                // Address
                showAddressDetail(query);
                return;
                
            } else if (!isNaN(query)) {
                // Block number
                const blockNum = parseInt(query);
                const block = await rpcCall('eth_getBlockByNumber', [`0x${blockNum.toString(16)}`, true]);
                if (block) {
                    showBlockDetailFromData(block);
                    return;
                }
            }
            
            alert('No results found. Please check your search query.');
            
        } catch (error) {
            console.error('Search error:', error);
            alert('Search failed. Please try again.');
        }
    };
    
    searchBtn.addEventListener('click', performSearch);
    searchInput.addEventListener('keypress', (e) => {
        if (e.key === 'Enter') performSearch();
    });
}

// ============================================================================
// DETAIL PANELS
// ============================================================================

function showDetailPanel(title, content) {
    const panel = document.getElementById('detail-panel');
    const titleEl = document.getElementById('detail-title');
    const contentEl = document.getElementById('detail-content');
    
    titleEl.textContent = title;
    contentEl.innerHTML = content;
    panel.classList.remove('hidden');
    panel.scrollIntoView({ behavior: 'smooth' });
}

function closeDetailPanel() {
    document.getElementById('detail-panel').classList.add('hidden');
}

async function showBlockDetail(hash) {
    try {
        const block = await rpcCall('eth_getBlockByHash', [hash, true]);
        if (block) {
            showBlockDetailFromData(block);
        }
    } catch (error) {
        console.error('Error loading block:', error);
    }
}

async function showBlockDetailFromData(block) {
    const blockNum = parseInt(block.number, 16);
    const timestamp = parseInt(block.timestamp, 16);
    const txCount = block.transactions ? block.transactions.length : 0;
    
    // Get parent hash(es) - support both single parentHash and multiple parentHashes
    const parentHashes = block.parentHashes || (block.parentHash && block.parentHash !== '0x0000000000000000000000000000000000000000000000000000000000000000' ? [block.parentHash] : []);
    
    // Fetch parent blocks
    const parentBlocks = [];
    for (const hash of parentHashes) {
        try {
            const parentBlock = await rpcCall('eth_getBlockByHash', [hash, false]);
            if (parentBlock) {
                parentBlocks.push(parentBlock);
            }
        } catch (e) {
            console.warn('Failed to fetch parent block:', hash, e);
        }
    }
    
    // Find child blocks by checking subsequent block numbers
    const childBlocks = [];
    try {
        const latestBlockHex = await rpcCall('eth_blockNumber');
        const latestBlock = parseInt(latestBlockHex, 16);
        
        // Check up to 5 blocks ahead for children
        for (let i = 1; i <= 5 && (blockNum + i) <= latestBlock; i++) {
            try {
                const childBlock = await rpcCall('eth_getBlockByNumber', [`0x${(blockNum + i).toString(16)}`, false]);
                if (childBlock) {
                    // Check if this block references our block as a parent
                    const childParentHashes = childBlock.parentHashes || [childBlock.parentHash];
                    if (childParentHashes.some(h => h.toLowerCase() === block.hash.toLowerCase())) {
                        childBlocks.push(childBlock);
                    }
                }
            } catch (e) {
                console.warn('Failed to check for child block:', blockNum + i, e);
            }
        }
    } catch (e) {
        console.warn('Failed to fetch latest block number for child search:', e);
    }
    
    // Stream type and consensus status
    const streamType = getStreamType(block);
    const streamBadge = `<span class="badge badge-stream-${streamType.toLowerCase()}">${streamType}</span>`;
    const isBlue = (latestBlockNumber - blockNum) >= 10;
    const consensusStatus = isBlue 
        ? '<span class="consensus-dot consensus-blue"></span><span class="text-success">Finalized</span>'
        : '<span class="consensus-dot consensus-red"></span><span class="text-pending">Pending</span>';
    
    // Parent links (all parents)
    const parentLinks = parentHashes.map(h => 
        `<a href="#" onclick="showBlockDetail('${h}'); return false;" class="hash-link">${h.substring(0,18)}...</a>`
    ).join('<br>');
    
    const content = `
        <div class="detail-row">
            <span class="detail-label">Block Number</span>
            <span class="detail-value">${blockNum} ${streamBadge}</span>
        </div>
        <div class="detail-row">
            <span class="detail-label">Consensus Status</span>
            <span class="detail-value">${consensusStatus}</span>
        </div>
        <div class="detail-row">
            <span class="detail-label">Hash</span>
            <span class="detail-value">${block.hash}</span>
        </div>
        <div class="detail-row">
            <span class="detail-label">Timestamp</span>
            <span class="detail-value">${new Date(timestamp * 1000).toLocaleString()} (<span data-timestamp="${timestamp}">${timeAgo(timestamp)}</span>)</span>
        </div>
        <div class="detail-row">
            <span class="detail-label">Transactions</span>
            <span class="detail-value">${txCount}</span>
        </div>
        <div class="detail-row">
            <span class="detail-label">Miner</span>
            <span class="detail-value link" onclick="showAddressDetail('${block.miner}')">${block.miner}</span>
        </div>
        <div class="detail-row">
            <span class="detail-label">Parent Hash${parentHashes.length > 1 ? 'es' : ''}</span>
            <span class="detail-value">${parentLinks || 'None (Genesis)'}</span>
        </div>
        <div class="detail-row">
            <span class="detail-label">Gas Used</span>
            <span class="detail-value">${parseInt(block.gasUsed || '0x0', 16)}</span>
        </div>
        <div class="detail-row">
            <span class="detail-label">Gas Limit</span>
            <span class="detail-value">${parseInt(block.gasLimit || '0x0', 16)}</span>
        </div>
        
        <!-- DAG Visualization Section -->
        <div class="dag-visualization">
            <h3><i class="fas fa-project-diagram"></i> Block DAG</h3>
            <p class="dag-description">Visualizing parent-child relationships in the BlockDAG structure</p>
            <div class="dag-container" id="mini-dag-container" style="height: 280px;">
            </div>
            <div style="text-align: center; margin-top: 12px;">
                <button class="dag-btn" onclick="highlightInDag('${block.hash}')"><i class="fas fa-eye"></i> View in DAG</button>
            </div>
        </div>
    `;
    
    showDetailPanel(`Block #${blockNum}`, content);
    
    // Render mini Cytoscape DAG after the panel is shown
    setTimeout(() => {
        renderMiniDag('mini-dag-container', block, parentBlocks, childBlocks);
    }, 100);
}

/**
 * Render DAG visualization as inline SVG
 * @param {Object} currentBlock - The currently selected block
 * @param {Array} parentBlocks - Array of parent block objects
 * @param {Array} childBlocks - Array of child block objects
 * @returns {string} - HTML string containing the SVG
 */
function renderDagVisualization(currentBlock, parentBlocks, childBlocks) {
    const currentNum = parseInt(currentBlock.number, 16);
    const parentCount = parentBlocks.length;
    const childCount = childBlocks.length;
    
    // Handle genesis block (no parents)
    if (parentCount === 0 && childCount === 0) {
        return `
            <div class="dag-empty">
                <i class="fas fa-cube"></i>
                <div>Genesis Block</div>
                <div class="dag-empty-subtext">No parent or child blocks</div>
            </div>
        `;
    }
    
    if (parentCount === 0) {
        return `
            <div class="dag-empty">
                <i class="fas fa-cube"></i>
                <div>Genesis Block</div>
                <div class="dag-empty-subtext">This is the first block in the chain</div>
            </div>
            <div class="dag-children-list">
                <div class="dag-label">Children:</div>
                ${childBlocks.map(b => `
                    <div class="dag-node-link" onclick="navigateToDagBlock('${b.hash}')">
                        <i class="fas fa-cube"></i> Block ${parseInt(b.number, 16)}
                    </div>
                `).join('')}
            </div>
        `;
    }
    
    if (childCount === 0) {
        return `
            <div class="dag-parents-list">
                <div class="dag-label">Parents:</div>
                ${parentBlocks.map(b => `
                    <div class="dag-node-link" onclick="navigateToDagBlock('${b.hash}')">
                        <i class="fas fa-cube"></i> Block ${parseInt(b.number, 16)}
                    </div>
                `).join('')}
            </div>
            <div class="dag-empty">
                <i class="fas fa-cube"></i>
                <div>Tip Block</div>
                <div class="dag-empty-subtext">No children yet (latest block in this chain)</div>
            </div>
        `;
    }
    
    // Calculate SVG dimensions based on number of nodes
    const maxNodesInRow = Math.max(parentCount, childCount, 1);
    const svgWidth = Math.max(400, maxNodesInRow * 140);
    const svgHeight = 280;
    const nodeWidth = 110;
    const nodeHeight = 50;
    const nodeRadius = 8;
    
    // Calculate positions
    const parentY = 40;
    const currentY = 140;
    const childY = 240;
    const centerX = svgWidth / 2;
    
    // Helper to calculate X positions for a row of nodes
    function calculateRowPositions(count, y) {
        const positions = [];
        const totalWidth = count * nodeWidth + (count - 1) * 20;
        const startX = centerX - totalWidth / 2 + nodeWidth / 2;
        for (let i = 0; i < count; i++) {
            positions.push({
                x: startX + i * (nodeWidth + 20),
                y: y
            });
        }
        return positions;
    }
    
    const parentPositions = calculateRowPositions(parentCount, parentY);
    const childPositions = calculateRowPositions(childCount, childY);
    const currentPosition = { x: centerX, y: currentY };
    
    // Build SVG
    let svg = `
        <svg viewBox="0 0 ${svgWidth} ${svgHeight}" class="dag-svg" preserveAspectRatio="xMidYMid meet">
            <defs>
                <marker id="arrowhead" markerWidth="10" markerHeight="7" refX="9" refY="3.5" orient="auto">
                    <polygon points="0 0, 10 3.5, 0 7" fill="#8b5cf6" />
                </marker>
                <filter id="glow">
                    <feGaussianBlur stdDeviation="2" result="coloredBlur"/>
                    <feMerge>
                        <feMergeNode in="coloredBlur"/>
                        <feMergeNode in="SourceGraphic"/>
                    </feMerge>
                </filter>
            </defs>
    `;
    
    // Draw edges from parents to current
    for (let i = 0; i < parentCount; i++) {
        const parent = parentBlocks[i];
        const parentPos = parentPositions[i];
        const startX = parentPos.x;
        const startY = parentPos.y + nodeHeight / 2;
        const endX = currentPosition.x;
        const endY = currentPosition.y - nodeHeight / 2;
        
        // Draw curved path
        svg += `
            <path d="M ${startX} ${startY} 
                     Q ${startX} ${(startY + endY) / 2}, ${(startX + endX) / 2} ${(startY + endY) / 2}
                     T ${endX} ${endY}"
                  stroke="#8b5cf6" stroke-width="2" fill="none" marker-end="url(#arrowhead)"
                  class="dag-edge" />
        `;
    }
    
    // Draw edges from current to children
    for (let i = 0; i < childCount; i++) {
        const child = childBlocks[i];
        const childPos = childPositions[i];
        const startX = currentPosition.x;
        const startY = currentPosition.y + nodeHeight / 2;
        const endX = childPos.x;
        const endY = childPos.y - nodeHeight / 2;
        
        svg += `
            <path d="M ${startX} ${startY} 
                     Q ${startX} ${(startY + endY) / 2}, ${(startX + endX) / 2} ${(startY + endY) / 2}
                     T ${endX} ${endY}"
                  stroke="#8b5cf6" stroke-width="2" fill="none" marker-end="url(#arrowhead)"
                  class="dag-edge" />
        `;
    }
    
    // Draw parent nodes
    for (let i = 0; i < parentCount; i++) {
        const parent = parentBlocks[i];
        const pos = parentPositions[i];
        const parentNum = parseInt(parent.number, 16);
        const hashShort = truncateHash(parent.hash, 8, 4);
        
        svg += `
            <g class="dag-node dag-node-parent" onclick="navigateToDagBlock('${parent.hash}')" style="cursor: pointer;">
                <rect x="${pos.x - nodeWidth/2}" y="${pos.y - nodeHeight/2}" 
                      width="${nodeWidth}" height="${nodeHeight}" rx="${nodeRadius}" ry="${nodeRadius}"
                      fill="#272439" stroke="#8b5cf6" stroke-width="2" />
                <text x="${pos.x}" y="${pos.y - 8}" text-anchor="middle" 
                      fill="#ffffff" font-size="12" font-weight="600">#${parentNum}</text>
                <text x="${pos.x}" y="${pos.y + 10}" text-anchor="middle" 
                      fill="#b8b4d0" font-size="9" font-family="Courier New, monospace">${hashShort}</text>
            </g>
        `;
    }
    
    // Draw current block node (highlighted)
    svg += `
        <g class="dag-node dag-node-current" filter="url(#glow)">
            <rect x="${currentPosition.x - nodeWidth/2}" y="${currentPosition.y - nodeHeight/2}" 
                  width="${nodeWidth}" height="${nodeHeight}" rx="${nodeRadius}" ry="${nodeRadius}"
                  fill="#6366f1" stroke="#8b5cf6" stroke-width="3" />
            <text x="${currentPosition.x}" y="${currentPosition.y - 8}" text-anchor="middle" 
                  fill="#ffffff" font-size="13" font-weight="700">#${currentNum}</text>
            <text x="${currentPosition.x}" y="${currentPosition.y + 10}" text-anchor="middle" 
                  fill="#ffffff" font-size="9" font-family="Courier New, monospace">${truncateHash(currentBlock.hash, 8, 4)}</text>
        </g>
    `;
    
    // Draw child nodes
    for (let i = 0; i < childCount; i++) {
        const child = childBlocks[i];
        const pos = childPositions[i];
        const childNum = parseInt(child.number, 16);
        const hashShort = truncateHash(child.hash, 8, 4);
        
        svg += `
            <g class="dag-node dag-node-child" onclick="navigateToDagBlock('${child.hash}')" style="cursor: pointer;">
                <rect x="${pos.x - nodeWidth/2}" y="${pos.y - nodeHeight/2}" 
                      width="${nodeWidth}" height="${nodeHeight}" rx="${nodeRadius}" ry="${nodeRadius}"
                      fill="#272439" stroke="#8b5cf6" stroke-width="2" />
                <text x="${pos.x}" y="${pos.y - 8}" text-anchor="middle" 
                      fill="#ffffff" font-size="12" font-weight="600">#${childNum}</text>
                <text x="${pos.x}" y="${pos.y + 10}" text-anchor="middle" 
                      fill="#b8b4d0" font-size="9" font-family="Courier New, monospace">${hashShort}</text>
            </g>
        `;
    }
    
    svg += '</svg>';
    return svg;
}

/**
 * Render a mini Cytoscape DAG for block detail view
 * @param {string} containerId - ID of the container element
 * @param {Object} currentBlock - The currently selected block
 * @param {Array} parentBlocks - Array of parent block objects
 * @param {Array} childBlocks - Array of child block objects
 */
function renderMiniDag(containerId, currentBlock, parentBlocks, childBlocks) {
    const container = document.getElementById(containerId);
    if (!container || typeof cytoscape === 'undefined') {
        console.log('Mini DAG container not found or cytoscape not loaded');
        if (container) {
            container.innerHTML = '<div style="text-align:center; padding:40px; color:#94a3b8;">DAG visualization unavailable</div>';
        }
        return;
    }

    const elements = [];
    const currentNum = parseInt(currentBlock.number, 16);
    const currentStreamType = getStreamType(currentBlock);
    
    // Current block node
    elements.push({
        data: {
            id: currentBlock.hash,
            label: '#' + currentNum,
            streamType: currentStreamType,
            isBlue: true,
            isCurrent: true
        },
        classes: 'current-block'
    });
    
    // Parent nodes and edges
    for (const p of parentBlocks) {
        if (p) {
            const parentNum = parseInt(p.number, 16);
            const parentStreamType = getStreamType(p);
            elements.push({
                data: {
                    id: p.hash,
                    label: '#' + parentNum,
                    streamType: parentStreamType,
                    isBlue: true
                }
            });
            elements.push({
                data: {
                    id: currentBlock.hash + '->' + p.hash,
                    source: currentBlock.hash,
                    target: p.hash
                }
            });
        }
    }
    
    // Child nodes and edges
    for (const c of childBlocks) {
        if (c) {
            const childNum = parseInt(c.number, 16);
            const childStreamType = getStreamType(c);
            elements.push({
                data: {
                    id: c.hash,
                    label: '#' + childNum,
                    streamType: childStreamType,
                    isBlue: true
                }
            });
            elements.push({
                data: {
                    id: c.hash + '->' + currentBlock.hash,
                    source: c.hash,
                    target: currentBlock.hash
                }
            });
        }
    }
    
    // Handle empty case
    if (elements.length === 1) {
        container.innerHTML = `
            <div class="dag-empty">
                <i class="fas fa-cube"></i>
                <div>Genesis Block</div>
                <div class="dag-empty-subtext">No parent or child blocks</div>
            </div>
        `;
        return;
    }
    
    // Initialize mini Cytoscape
    try {
        const miniCy = cytoscape({
            container: container,
            elements: elements,
            style: [
                {
                    selector: 'node',
                    style: {
                        'label': 'data(label)',
                        'text-valign': 'center',
                        'text-halign': 'center',
                        'font-size': '10px',
                        'color': '#e2e8f0',
                        'text-outline-color': '#1e1b2e',
                        'text-outline-width': 2,
                        'width': 45,
                        'height': 45,
                        'border-width': 3
                    }
                },
                {
                    selector: 'node[streamType = "A"]',
                    style: { 'background-color': '#3b82f6' }
                },
                {
                    selector: 'node[streamType = "B"]',
                    style: { 'background-color': '#8b5cf6' }
                },
                {
                    selector: 'node[streamType = "C"]',
                    style: { 'background-color': '#10b981' }
                },
                {
                    selector: 'node[?isBlue]',
                    style: { 'border-color': '#10b981' }
                },
                {
                    selector: 'node[!isBlue]',
                    style: { 'border-color': '#f59e0b' }
                },
                {
                    selector: 'node.current-block',
                    style: {
                        'border-width': 5,
                        'border-color': '#fbbf24',
                        'font-weight': 'bold'
                    }
                },
                {
                    selector: 'edge',
                    style: {
                        'width': 2,
                        'line-color': '#4a4660',
                        'target-arrow-color': '#4a4660',
                        'target-arrow-shape': 'triangle',
                        'curve-style': 'bezier',
                        'arrow-scale': 0.8,
                        'opacity': 0.7
                    }
                }
            ],
            layout: {
                name: 'breadthfirst',
                directed: true,
                spacingFactor: 1.5,
                avoidOverlap: true,
                rankDir: 'LR'
            },
            userZoomingEnabled: false,
            userPanningEnabled: false,
            autoungrabify: true,
            minZoom: 0.5,
            maxZoom: 2
        });
        
        // Click handler to navigate to other blocks
        miniCy.on('tap', 'node', function(evt) {
            const hash = evt.target.id();
            if (hash !== currentBlock.hash) {
                showBlockDetail(hash);
            }
        });
        
        // Fit the view
        miniCy.fit(undefined, 20);
        
    } catch (err) {
        console.error('Mini DAG render error:', err);
        container.innerHTML = '<div style="text-align:center; padding:40px; color:#94a3b8;">DAG visualization error</div>';
    }
}

/**
 * Navigate to a different block from the DAG visualization
 * @param {string} hash - Block hash to navigate to
 */
async function navigateToDagBlock(hash) {
    try {
        const block = await rpcCall('eth_getBlockByHash', [hash, true]);
        if (block) {
            showBlockDetailFromData(block);
        }
    } catch (error) {
        console.error('Error navigating to block:', error);
    }
}

async function showTxDetail(hash) {
    try {
        const tx = await rpcCall('eth_getTransactionByHash', [hash]);
        if (!tx) {
            alert('Transaction not found');
            return;
        }
        
        // Get actual transaction status from receipt
        const status = await getTransactionStatus(hash);
        const statusDisplay = status === 'success' 
            ? '<span class="detail-value text-success"><i class="fas fa-check"></i> Success</span>'
            : status === 'failed'
            ? '<span class="detail-value text-failed"><i class="fas fa-times"></i> Failed</span>'
            : '<span class="detail-value text-pending"><i class="fas fa-clock"></i> Pending</span>';
        
        const txType = getTransactionType(tx);
        const value = parseInt(tx.value || '0x0', 16) / 1e18;
        const gasPrice = parseInt(tx.gasPrice || '0x0', 16);
        const gas = parseInt(tx.gas || '0x0', 16);
        const fee = (gasPrice * gas) / 1e18;
        const blockNum = parseInt(tx.blockNumber, 16);
        
        const content = `
            <div class="detail-row">
                <span class="detail-label">Transaction Hash</span>
                <span class="detail-value">${tx.hash}</span>
            </div>
            <div class="detail-row">
                <span class="detail-label">Status</span>
                ${statusDisplay}
            </div>
            <div class="detail-row">
                <span class="detail-label">Type</span>
                <span class="detail-value">${txType.replace('-', ' ').toUpperCase()}</span>
            </div>
            <div class="detail-row">
                <span class="detail-label">Block</span>
                <span class="detail-value link" onclick="showBlockDetail('${tx.blockHash}')">${blockNum}</span>
            </div>
            <div class="detail-row">
                <span class="detail-label">From</span>
                <span class="detail-value link" onclick="showAddressDetail('${tx.from}')">${tx.from}</span>
            </div>
            <div class="detail-row">
                <span class="detail-label">To</span>
                <span class="detail-value link" onclick="showAddressDetail('${tx.to || ''}')">${tx.to || 'Contract Creation'}</span>
            </div>
            <div class="detail-row">
                <span class="detail-label">Value</span>
                <span class="detail-value">${value.toFixed(6)} IDAG</span>
            </div>
            <div class="detail-row">
                <span class="detail-label">Transaction Fee</span>
                <span class="detail-value">${fee.toFixed(8)} IDAG</span>
            </div>
            <div class="detail-row">
                <span class="detail-label">Gas Price</span>
                <span class="detail-value">${(gasPrice / 1e9).toFixed(2)} Gwei</span>
            </div>
            <div class="detail-row">
                <span class="detail-label">Gas Limit</span>
                <span class="detail-value">${gas}</span>
            </div>
            <div class="detail-row">
                <span class="detail-label">Nonce</span>
                <span class="detail-value">${parseInt(tx.nonce, 16)}</span>
            </div>
            ${tx.input && tx.input !== '0x' ? `
            <div class="detail-row">
                <span class="detail-label">Input Data</span>
                <span class="detail-value" style="word-break: break-all; font-size: 0.8rem;">${tx.input}</span>
            </div>
            ` : ''}
        `;
        
        showDetailPanel('Transaction Details', content);
        
    } catch (error) {
        console.error('Error loading transaction:', error);
        alert('Error loading transaction details');
    }
}

async function showAddressDetail(address) {
    if (!address || address === 'Contract') return;

    try {
        // Get balance
        const balance = await rpcCall('eth_getBalance', [address, 'latest']).catch(() => '0x0');
        const balanceEth = parseInt(balance, 16) / 1e18;

        // Get transaction count (nonce)
        const nonce = await rpcCall('eth_getTransactionCount', [address, 'latest']).catch(() => '0x0');
        const txCount = parseInt(nonce, 16);

        // Check if contract
        const code = await rpcCall('eth_getCode', [address, 'latest']).catch(() => '0x');
        const isContract = code && code !== '0x' && code.length > 2;

        const content = `
            <div class="detail-row">
                <span class="detail-label">Address</span>
                <span class="detail-value">${address}</span>
            </div>
            <div class="detail-row">
                <span class="detail-label">Type</span>
                <span class="detail-value">${isContract ? 'Contract' : 'Externally Owned Account'}</span>
            </div>
            <div class="detail-row">
                <span class="detail-label">Balance</span>
                <span class="detail-value">${balanceEth.toFixed(6)} IDAG</span>
            </div>
            <div class="detail-row">
                <span class="detail-label">Transactions</span>
                <span class="detail-value">${txCount}</span>
            </div>
            ${isContract ? `
            <div class="detail-row">
                <span class="detail-label">Contract Code</span>
                <span class="detail-value" style="word-break: break-all; font-size: 0.75rem; max-height: 100px; overflow-y: auto;">${code.slice(0, 200)}${code.length > 200 ? '...' : ''}</span>
            </div>
            ` : ''}
            <!-- Transaction History Section -->
            <div class="address-tx-history" id="address-tx-history">
                <h3><i class="fas fa-exchange-alt"></i> Transaction History</h3>
                <div id="address-tx-content">
                    <div class="address-tx-loading">
                        <i class="fas fa-spinner fa-spin"></i>
                        <span>Scanning blocks for transactions...</span>
                    </div>
                </div>
            </div>
        `;

        showDetailPanel('Address Details', content);

        // Load transaction history asynchronously
        loadAddressTransactions(address, 1);

    } catch (error) {
        console.error('Error loading address:', error);
        alert('Error loading address details');
    }
}

// ============================================================================
// ADDRESS TRANSACTION HISTORY
// ============================================================================

/**
 * Scan blocks to find transactions involving an address
 * @param {string} address - The address to find transactions for
 * @param {number} page - Page number (1-indexed)
 */
async function loadAddressTransactions(address, page = 1) {
    const container = document.getElementById('address-tx-content');
    if (!container) return;

    addressTxState.currentPage = page;

    // Check cache
    const now = Date.now();
    const cached = addressTxState.cache.get(address);

    // Use cache if valid
    if (cached && (now - cached.timestamp) < addressTxState.cacheTimeout) {
        renderAddressTransactions(address, cached.transactions, page);
        return;
    }

    try {
        // Get current block number
        const blockNumberHex = await rpcCall('eth_blockNumber');
        const latestBlock = parseInt(blockNumberHex, 16);

        const transactions = [];
        let blocksScanned = 0;
        const maxBlocks = addressTxState.maxBlocksToScan;
        const startBlock = latestBlock;
        const endBlock = Math.max(0, latestBlock - maxBlocks + 1);

        // Scan blocks in batches for efficiency
        const batchSize = 10;
        for (let batchStart = startBlock; batchStart >= endBlock; batchStart -= batchSize) {
            const batchEnd = Math.max(endBlock, batchStart - batchSize + 1);
            const blockPromises = [];

            for (let blockNum = batchStart; blockNum >= batchEnd; blockNum--) {
                blockPromises.push(
                    rpcCall('eth_getBlockByNumber', [`0x${blockNum.toString(16)}`, true])
                        .then(block => {
                            if (block && block.transactions && block.transactions.length > 0) {
                                const matchingTxs = block.transactions.filter(tx =>
                                    tx.from?.toLowerCase() === address.toLowerCase() ||
                                    tx.to?.toLowerCase() === address.toLowerCase()
                                );
                                return matchingTxs.map(tx => ({
                                    ...tx,
                                    blockNumber: parseInt(block.number, 16),
                                    blockTimestamp: parseInt(block.timestamp, 16),
                                    direction: tx.from?.toLowerCase() === address.toLowerCase() ? 'out' : 'in'
                                }));
                            }
                            return [];
                        })
                        .catch(() => [])
                );
            }

            const batchResults = await Promise.all(blockPromises);
            for (const txs of batchResults) {
                transactions.push(...txs);
            }

            blocksScanned += batchSize;

            // Update loading indicator with progress
            if (blocksScanned % 50 === 0) {
                container.innerHTML = `
                    <div class="address-tx-loading">
                        <i class="fas fa-spinner fa-spin"></i>
                        <span>Scanning blocks... ${Math.min(blocksScanned, maxBlocks)}/${maxBlocks} (${transactions.length} txs found)</span>
                    </div>
                `;
            }

            // Stop if we have enough transactions for display
            if (transactions.length >= 100) break;
        }

        // Sort by block number descending (newest first)
        transactions.sort((a, b) => b.blockNumber - a.blockNumber);

        // Cache results
        addressTxState.cache.set(address, {
            transactions,
            scannedToBlock: endBlock,
            timestamp: now
        });

        renderAddressTransactions(address, transactions, page);

    } catch (error) {
        console.error('Error loading address transactions:', error);
        container.innerHTML = `
            <div class="address-tx-empty">
                <i class="fas fa-exclamation-triangle"></i>
                <div>Error loading transactions</div>
            </div>
        `;
    }
}

/**
 * Render the transaction history list
 * @param {string} address - The address being viewed
 * @param {Array} transactions - Array of transaction objects
 * @param {number} page - Current page number
 */
function renderAddressTransactions(address, transactions, page) {
    const container = document.getElementById('address-tx-content');
    if (!container) return;

    const pageSize = addressTxState.pageSize;
    const startIndex = (page - 1) * pageSize;
    const endIndex = startIndex + pageSize;
    const pageTransactions = transactions.slice(startIndex, endIndex);
    const totalPages = Math.ceil(transactions.length / pageSize);

    // Calculate summary stats
    let totalIn = 0;
    let totalOut = 0;

    for (const tx of transactions) {
        const value = parseInt(tx.value || '0x0', 16) / 1e18;
        if (tx.direction === 'in') {
            totalIn += value;
        } else {
            totalOut += value;
        }
    }

    const netChange = totalIn - totalOut;

    if (transactions.length === 0) {
        container.innerHTML = `
            <div class="address-tx-empty">
                <i class="fas fa-inbox"></i>
                <div>No transactions found in the last ${addressTxState.maxBlocksToScan} blocks</div>
            </div>
        `;
        return;
    }

    let html = `
        <div class="address-tx-summary">
            <div class="address-tx-summary-item">
                <span class="address-tx-summary-label">Total In</span>
                <span class="address-tx-summary-value incoming">${totalIn.toFixed(6)} IDAG</span>
            </div>
            <div class="address-tx-summary-item">
                <span class="address-tx-summary-label">Total Out</span>
                <span class="address-tx-summary-value outgoing">${totalOut.toFixed(6)} IDAG</span>
            </div>
            <div class="address-tx-summary-item">
                <span class="address-tx-summary-label">Net Change</span>
                <span class="address-tx-summary-value ${netChange >= 0 ? 'incoming' : 'outgoing'}">${netChange >= 0 ? '+' : ''}${netChange.toFixed(6)} IDAG</span>
            </div>
        </div>
        <div class="address-tx-note">
            <i class="fas fa-info-circle"></i> Showing transactions from last ${addressTxState.maxBlocksToScan} blocks
        </div>
        <div class="address-tx-list">
    `;

    for (const tx of pageTransactions) {
        const value = parseInt(tx.value || '0x0', 16) / 1e18;
        const counterparty = tx.direction === 'in' ? tx.from : tx.to;
        const directionIcon = tx.direction === 'in' ? 'fa-arrow-down' : 'fa-arrow-up';
        const directionText = tx.direction === 'in' ? 'IN' : 'OUT';

        html += `
            <div class="address-tx-item">
                <div class="address-tx-direction ${tx.direction}">
                    <i class="fas ${directionIcon}"></i>
                </div>
                <div class="address-tx-content">
                    <div class="address-tx-main">
                        <span class="address-tx-counterparty" onclick="showAddressDetail('${counterparty}')" title="${counterparty}">
                            ${counterparty ? truncateAddress(counterparty) : 'Contract Creation'}
                        </span>
                        <span class="address-tx-value ${tx.direction === 'in' ? 'incoming' : 'outgoing'}">
                            ${tx.direction === 'in' ? '+' : '-'}${value.toFixed(6)} IDAG
                        </span>
                    </div>
                    <div class="address-tx-meta">
                        <span class="address-tx-hash" onclick="showTxDetail('${tx.hash}')" title="${tx.hash}">
                            ${truncateHash(tx.hash)}
                        </span>
                        <span class="address-tx-block">Block ${tx.blockNumber}</span>
                    </div>
                </div>
                <div class="address-tx-time">
                    <span data-timestamp="${tx.blockTimestamp}">${timeAgo(tx.blockTimestamp)}</span>
                </div>
            </div>
        `;
    }

    html += '</div>';

    // Add pagination if needed
    if (totalPages > 1) {
        html += `
            <div class="address-tx-pagination">
                <div class="pagination">
                    <button class="page-btn" onclick="prevAddressTxPage('${address}')" ${page <= 1 ? 'disabled' : ''}>← Prev</button>
                    <span class="page-info">Page ${page} of ${totalPages} (${transactions.length} txs)</span>
                    <button class="page-btn" onclick="nextAddressTxPage('${address}')" ${page >= totalPages ? 'disabled' : ''}>Next →</button>
                </div>
            </div>
        `;
    }

    container.innerHTML = html;
}

/**
 * Navigate to previous page in address transaction history
 */
function prevAddressTxPage(address) {
    if (addressTxState.currentPage > 1) {
        const cached = addressTxState.cache.get(address);
        if (cached) {
            renderAddressTransactions(address, cached.transactions, addressTxState.currentPage - 1);
        }
    }
}

/**
 * Navigate to next page in address transaction history
 */
function nextAddressTxPage(address) {
    const cached = addressTxState.cache.get(address);
    if (cached) {
        const totalPages = Math.ceil(cached.transactions.length / addressTxState.pageSize);
        if (addressTxState.currentPage < totalPages) {
            renderAddressTransactions(address, cached.transactions, addressTxState.currentPage + 1);
        }
    }
}

// ============================================================================
// INITIALIZATION
// ============================================================================

document.addEventListener('DOMContentLoaded', () => {
    // Initial load
    loadDashboard();
    loadRecentBlocks();
    loadRecentTransactions();
    setupSearch();
    setupFaucet();
    setupWalletConnection();
    initDagVisualization();
    
    // Setup stream filter tabs
    document.querySelectorAll('.stream-tab').forEach(tab => {
        tab.addEventListener('click', () => {
            document.querySelectorAll('.stream-tab').forEach(t => t.classList.remove('active'));
            tab.classList.add('active');
            activeStreamFilter = tab.dataset.stream;
            // Reset pagination to page 1 when filter changes
            paginationState.blocks.page = 1;
            loadRecentBlocks();
        });
    });
    
    // Auto-refresh data every 10 seconds (increased from 5s to prevent rate limit flooding)
    setInterval(() => {
        loadDashboard();
        loadRecentBlocks();
        loadRecentTransactions();
        if (window.connectedAddress) updateWalletDisplay();
    }, 10000);

    // Refresh DAG every 15 seconds using incremental updates
    setInterval(() => {
        updateDagVisualization();
    }, DAG_REFRESH_INTERVAL);

    // Live-update relative times every second (blocks/tx "X ago" displays)
    setInterval(updateLiveTimes, 1000);
});

// ============================================================================
// FAUCET FUNCTIONALITY
// ============================================================================

function setupFaucet() {
    const faucetBtn = document.getElementById('faucet-btn');
    const faucetInput = document.getElementById('faucet-address');
    const faucetStatus = document.getElementById('faucet-status');
    
    if (!faucetBtn) return;
    
    faucetBtn.addEventListener('click', async () => {
        const address = faucetInput.value.trim();
        
        // Validate address
        if (!address) {
            showFaucetStatus('Please enter a wallet address', 'error');
            return;
        }
        
        if (!address.match(/^0x[a-fA-F0-9]{40}$/)) {
            showFaucetStatus('Invalid address format. Must be 0x followed by 40 hex characters.', 'error');
            return;
        }
        
        // Disable button during request
        faucetBtn.disabled = true;
        showFaucetStatus('Requesting tokens...', 'pending');
        
        try {
            // Call faucet RPC method
            const result = await rpcCall('irondag_faucet', [address]);
            
            if (result && result.txHash) {
                showFaucetStatus(`Success! 10 IDAG sent. TX: ${truncateHash(result.txHash)}`, 'success');
                faucetInput.value = '';
            } else if (result && result.error) {
                showFaucetStatus(result.error, 'error');
            } else {
                showFaucetStatus('Tokens sent successfully!', 'success');
                faucetInput.value = '';
            }
        } catch (error) {
            // Check if faucet is not available in production
            if (error.message && error.message.includes('Method not available')) {
                showFaucetStatus('Faucet not available in production', 'error');
                // Hide faucet UI since it's not available
                const faucetContainer = document.getElementById('faucet-container');
                if (faucetContainer) faucetContainer.classList.add('hidden');
            } else {
                console.error('Faucet error:', error);
                showFaucetStatus(`Error: ${error.message || 'Failed to request tokens'}`, 'error');
            }
        } finally {
            faucetBtn.disabled = false;
        }
    });
    
    // Allow Enter key to submit
    faucetInput.addEventListener('keypress', (e) => {
        if (e.key === 'Enter') {
            faucetBtn.click();
        }
    });
}

function showFaucetStatus(message, type) {
    const faucetStatus = document.getElementById('faucet-status');
    faucetStatus.textContent = message;
    faucetStatus.className = `faucet-status ${type}`;
}

// Make functions globally available
window.showBlockDetail = showBlockDetail;
window.showTxDetail = showTxDetail;
window.showAddressDetail = showAddressDetail;
window.copyToClipboard = copyToClipboard;
window.closeDetailPanel = closeDetailPanel;
window.connectWallet = connectWallet;
window.disconnectWallet = disconnectWallet;
window.useConnectedWalletForFaucet = useConnectedWalletForFaucet;
window.prevPage = prevPage;
window.nextPage = nextPage;
window.changePageSize = changePageSize;
window.prevAddressTxPage = prevAddressTxPage;
window.nextAddressTxPage = nextAddressTxPage;
window.navigateToDagBlock = navigateToDagBlock;
window.highlightInDag = highlightInDag;

// ============================================================================
// METAMASK WALLET INTEGRATION
// ============================================================================

// Global state for connected wallet
window.connectedAddress = null;

/**
 * Request account access via MetaMask (window.ethereum)
 */
async function connectWallet() {
    if (typeof window.ethereum === 'undefined') {
        alert('MetaMask is not installed. Please install MetaMask to connect your wallet.');
        return;
    }

    try {
        const accounts = await window.ethereum.request({ method: 'eth_requestAccounts' });

        if (accounts.length > 0) {
            window.connectedAddress = accounts[0];
            localStorage.setItem('walletConnected', 'true');
            // Show connected state immediately, then load balance async
            showWalletConnected();
            loadWalletBalance();
            updateDevToolsWalletState();
            console.log('Wallet connected:', window.connectedAddress);
        }
    } catch (error) {
        console.error('Failed to connect wallet:', error);
        if (error.code === 4001) {
            alert('Connection rejected. Please approve the connection request in MetaMask.');
        } else {
            alert('Failed to connect wallet: ' + error.message);
        }
    }
}

/**
 * Immediately flip the header UI to show connected state (no RPC needed).
 */
function showWalletConnected() {
    if (!window.connectedAddress) return;
    const connectBtn = document.getElementById('connect-wallet-btn');
    const connectedDiv = document.getElementById('wallet-connected');
    const addressEl = document.getElementById('wallet-address');
    const useWalletBtn = document.getElementById('use-wallet-btn');

    if (addressEl) {
        addressEl.textContent = truncateAddress(window.connectedAddress);
        addressEl.title = `${window.connectedAddress} (Click to copy)`;
    }
    if (connectBtn) connectBtn.classList.add('hidden');
    if (connectedDiv) connectedDiv.classList.remove('hidden');
    if (useWalletBtn) useWalletBtn.classList.remove('hidden');
}

/**
 * Load and display wallet balance from RPC (runs in background after connect).
 */
async function loadWalletBalance() {
    if (!window.connectedAddress) return;
    const balanceEl = document.getElementById('wallet-balance');
    if (!balanceEl) return;
    try {
        const balance = await rpcCall('eth_getBalance', [window.connectedAddress, 'latest'], { retries: 2, quiet: true }).catch(() => '0x0');
        const balanceEth = parseInt(balance, 16) / 1e18;
        balanceEl.textContent = `${balanceEth.toFixed(4)} IDAG`;
    } catch (_) {}
}

/**
 * Clear connected wallet state
 */
function disconnectWallet() {
    window.connectedAddress = null;
    localStorage.removeItem('walletConnected');

    const connectBtn = document.getElementById('connect-wallet-btn');
    const connectedDiv = document.getElementById('wallet-connected');
    const useWalletBtn = document.getElementById('use-wallet-btn');
    if (connectBtn) connectBtn.classList.remove('hidden');
    if (connectedDiv) connectedDiv.classList.add('hidden');
    if (useWalletBtn) useWalletBtn.classList.add('hidden');
    updateDevToolsWalletState();
    console.log('Wallet disconnected');
}

/**
 * Refresh address display and balance (called by auto-refresh interval).
 */
async function updateWalletDisplay() {
    if (!window.connectedAddress) return;
    showWalletConnected();
    loadWalletBalance();
}

/**
 * Auto-fill faucet input with connected wallet address
 */
function useConnectedWalletForFaucet() {
    if (!window.connectedAddress) {
        alert('No wallet connected. Please connect your wallet first.');
        return;
    }
    
    const faucetInput = document.getElementById('faucet-address');
    faucetInput.value = window.connectedAddress;
    faucetInput.focus();
}

/**
 * Set up wallet connection event listeners
 */
function setupWalletConnection() {
    if (typeof window.ethereum === 'undefined') {
        console.log('MetaMask not detected');
        return;
    }
    
    // Auto-reconnect if previously connected
    const wasConnected = localStorage.getItem('walletConnected');
    if (wasConnected === 'true') {
        // Check if still connected
        window.ethereum.request({ method: 'eth_accounts' })
            .then(accounts => {
                if (accounts.length > 0) {
                    window.connectedAddress = accounts[0];
                    updateWalletDisplay();
                    updateDevToolsWalletState();
                    console.log('Auto-reconnected wallet:', window.connectedAddress);
                } else {
                    localStorage.removeItem('walletConnected');
                }
            })
            .catch(err => {
                console.error('Auto-reconnect failed:', err);
                localStorage.removeItem('walletConnected');
            });
    }

    // Listen for account changes
    window.ethereum.on('accountsChanged', (accounts) => {
        if (accounts.length === 0) {
            disconnectWallet();
        } else if (accounts[0] !== window.connectedAddress) {
            window.connectedAddress = accounts[0];
            updateWalletDisplay();
            updateDevToolsWalletState();
            console.log('Account changed to:', window.connectedAddress);
        }
    });
    
    // Listen for chain changes
    window.ethereum.on('chainChanged', (chainId) => {
        console.log('Chain changed to:', chainId);
        // Refresh balance on chain change
        if (window.connectedAddress) {
            updateWalletDisplay();
        }
    });
}

// ============================================================================
// DEVELOPER TOOLS
// ============================================================================

const IRONDAG_CHAIN = {
    chainId: '0x36C3',
    chainName: 'IronDAG Testnet',
    nativeCurrency: { name: 'IDAG', symbol: 'IDAG', decimals: 18 },
    rpcUrls: ['https://explorer.irondag.io/rpc'],
    blockExplorerUrls: ['https://explorer.irondag.io']
};

function getFullRpcUrl() {
    return RPC_BASE.startsWith('/') ? `${window.location.origin}${RPC_BASE}` : RPC_BASE;
}

function switchDevTab(tab) {
    document.querySelectorAll('.dev-tab').forEach(btn => btn.classList.remove('active'));
    document.querySelectorAll('.dev-panel').forEach(panel => panel.classList.add('hidden'));

    const panelMap = { network: 'dev-panel-network', send: 'dev-panel-send', deploy: 'dev-panel-deploy', interact: 'dev-panel-interact' };
    const tabOrder = ['network', 'send', 'deploy', 'interact'];

    const tabBtns = document.querySelectorAll('.dev-tab');
    const idx = tabOrder.indexOf(tab);
    if (idx >= 0 && tabBtns[idx]) tabBtns[idx].classList.add('active');

    const panel = document.getElementById(panelMap[tab]);
    if (panel) panel.classList.remove('hidden');

    updateDevToolsWalletState();
}

function updateDevToolsWalletState() {
    const connected = !!window.connectedAddress;

    const sendNotice = document.getElementById('send-wallet-notice');
    const sendFrom = document.getElementById('send-from-display');
    if (sendNotice) sendNotice.style.display = connected ? 'none' : 'flex';
    if (sendFrom) sendFrom.textContent = connected ? truncateAddress(window.connectedAddress) : 'Not connected';

    const deployNotice = document.getElementById('deploy-wallet-notice');
    if (deployNotice) deployNotice.style.display = connected ? 'none' : 'flex';

    const interactNotice = document.getElementById('interact-wallet-notice');
    if (interactNotice) interactNotice.style.display = connected ? 'none' : 'flex';
}

async function addToMetaMask() {
    const resultEl = document.getElementById('network-result');
    if (typeof window.ethereum === 'undefined') {
        setDevToolResult(resultEl, 'MetaMask is not installed. Please install MetaMask first.', 'error');
        return;
    }
    try {
        await window.ethereum.request({ method: 'wallet_addEthereumChain', params: [IRONDAG_CHAIN] });
        setDevToolResult(resultEl, 'IronDAG Testnet added to MetaMask successfully.', 'success');
    } catch (err) {
        setDevToolResult(resultEl, err.code === 4001 ? 'Request rejected by user.' : `Error: ${err.message}`, 'error');
    }
}

async function sendIDAG() {
    const resultEl = document.getElementById('send-result');
    if (!window.connectedAddress) {
        setDevToolResult(resultEl, 'Connect your wallet first.', 'error');
        return;
    }
    const to = document.getElementById('send-to').value.trim();
    const amount = document.getElementById('send-amount').value.trim();

    if (!to || !/^0x[0-9a-fA-F]{40}$/.test(to)) {
        setDevToolResult(resultEl, 'Enter a valid recipient address (0x...)', 'error');
        return;
    }
    if (!amount || isNaN(parseFloat(amount)) || parseFloat(amount) <= 0) {
        setDevToolResult(resultEl, 'Enter a valid amount greater than 0.', 'error');
        return;
    }

    try {
        const weiHex = '0x' + BigInt(Math.round(parseFloat(amount) * 1e18)).toString(16);
        const txHash = await window.ethereum.request({
            method: 'eth_sendTransaction',
            params: [{ from: window.connectedAddress, to, value: weiHex, gas: '0x5208' }]
        });
        setDevToolResult(resultEl, `Transaction submitted: ${txHash}`, 'success');
    } catch (err) {
        setDevToolResult(resultEl, err.code === 4001 ? 'Transaction rejected by user.' : `Error: ${err.message}`, 'error');
    }
}

function onDeployABIChange() {
    const abiText = document.getElementById('deploy-abi').value.trim();
    const ctorSection = document.getElementById('deploy-ctor-section');
    const ctorInputs = document.getElementById('deploy-ctor-inputs');

    if (!abiText) { ctorSection.style.display = 'none'; return; }
    let abi;
    try { abi = JSON.parse(abiText); } catch (_) { ctorSection.style.display = 'none'; return; }

    const ctor = abi.find(item => item.type === 'constructor');
    if (!ctor || !ctor.inputs || ctor.inputs.length === 0) { ctorSection.style.display = 'none'; return; }

    ctorSection.style.display = 'block';
    ctorInputs.innerHTML = ctor.inputs.map((inp, i) => `
        <div class="form-group" style="margin-bottom:var(--spacing-sm)">
            <label class="form-label" for="ctor-arg-${i}">${inp.name || `arg${i}`} <span>(${inp.type})</span></label>
            <input type="text" id="ctor-arg-${i}" class="form-input abi-arg-input" placeholder="${inp.type}">
        </div>
    `).join('');
}

async function deployContract() {
    const resultEl = document.getElementById('deploy-result');
    if (!window.connectedAddress) {
        setDevToolResult(resultEl, 'Connect your wallet to deploy contracts.', 'error');
        return;
    }
    const bytecodeRaw = document.getElementById('deploy-bytecode').value.trim();
    if (!bytecodeRaw) { setDevToolResult(resultEl, 'Paste compiled bytecode first.', 'error'); return; }

    if (typeof ethers === 'undefined') {
        setDevToolResult(resultEl, 'ethers.js failed to load. Refresh and try again.', 'error');
        return;
    }

    const bytecode = bytecodeRaw.startsWith('0x') ? bytecodeRaw : '0x' + bytecodeRaw;
    const abiText = document.getElementById('deploy-abi').value.trim();

    try {
        setDevToolResult(resultEl, 'Waiting for wallet confirmation...', 'info');
        const provider = new ethers.BrowserProvider(window.ethereum);
        const signer = await provider.getSigner();

        if (abiText) {
            let abi;
            try { abi = JSON.parse(abiText); } catch (_) { setDevToolResult(resultEl, 'Invalid ABI JSON.', 'error'); return; }

            const ctor = abi.find(item => item.type === 'constructor');
            const ctorArgs = ctor && ctor.inputs
                ? ctor.inputs.map((inp, i) => parseArgValue(document.getElementById(`ctor-arg-${i}`)?.value?.trim() ?? '', inp.type))
                : [];

            const factory = new ethers.ContractFactory(abi, bytecode, signer);
            const contract = await factory.deploy(...ctorArgs);
            setDevToolResult(resultEl, `Deploying... TX: ${contract.deploymentTransaction()?.hash}`, 'info');
            await contract.waitForDeployment();
            setDevToolResult(resultEl, `Contract deployed at: ${await contract.getAddress()}`, 'success');
        } else {
            const tx = await signer.sendTransaction({ data: bytecode });
            setDevToolResult(resultEl, `Deploying... TX: ${tx.hash}`, 'info');
            const receipt = await tx.wait();
            setDevToolResult(resultEl, `Contract deployed at: ${receipt.contractAddress}`, 'success');
        }
    } catch (err) {
        const rejected = err.code === 4001 || err.code === 'ACTION_REJECTED';
        setDevToolResult(resultEl, rejected ? 'Deployment rejected by user.' : `Error: ${err.message}`, 'error');
    }
}

// State for interact panel
let _interactAbi = null;
let _interactAddress = null;

function loadContractABI() {
    const resultEl = document.getElementById('interact-load-result');
    const addr = document.getElementById('interact-address').value.trim();
    const abiText = document.getElementById('interact-abi').value.trim();

    if (!addr || !/^0x[0-9a-fA-F]{40}$/.test(addr)) {
        setDevToolResult(resultEl, 'Enter a valid contract address (0x...)', 'error');
        return;
    }
    if (!abiText) { setDevToolResult(resultEl, 'Paste the contract ABI first.', 'error'); return; }

    let abi;
    try { abi = JSON.parse(abiText); } catch (_) { setDevToolResult(resultEl, 'Invalid JSON in ABI field.', 'error'); return; }

    const fns = abi.filter(item => item.type === 'function');
    if (fns.length === 0) { setDevToolResult(resultEl, 'No functions found in ABI.', 'error'); return; }

    _interactAbi = abi;
    _interactAddress = addr;

    const readFns  = fns.filter(f => f.stateMutability === 'view' || f.stateMutability === 'pure');
    const writeFns = fns.filter(f => f.stateMutability !== 'view' && f.stateMutability !== 'pure');

    renderABIFunctions('read-fns-list', readFns, 'read');
    renderABIFunctions('write-fns-list', writeFns, 'write');

    document.getElementById('interact-functions-area').style.display = 'block';
    updateDevToolsWalletState();
    setDevToolResult(resultEl, `Loaded ${fns.length} function(s) — ${readFns.length} read, ${writeFns.length} write.`, 'success');
}

function renderABIFunctions(containerId, fns, kind) {
    const container = document.getElementById(containerId);
    if (!container) return;
    if (fns.length === 0) { container.innerHTML = '<div class="abi-empty">No functions</div>'; return; }

    container.innerHTML = fns.map((fn, i) => {
        const id = `${kind}-fn-${i}`;
        const badge = fn.stateMutability === 'payable' ? 'payable' : (fn.stateMutability || '');
        const inputs = (fn.inputs || []).map((inp, j) => `
            <div class="form-group" style="margin-bottom:var(--spacing-xs)">
                <label class="abi-arg-label">${inp.name || `arg${j}`} <span>(${inp.type})</span></label>
                <input type="text" id="${id}-arg-${j}" class="form-input abi-arg-input" placeholder="${inp.type}">
            </div>
        `).join('');
        const btn = kind === 'read'
            ? `<button class="abi-call-btn read-btn" onclick="callReadFn(${i})">Call</button>`
            : `<button class="abi-call-btn write-btn" onclick="callWriteFn(${i})">Send</button>`;
        return `
            <div class="abi-fn-item">
                <div class="abi-fn-header" onclick="toggleAbiFn('${id}')">
                    <span class="abi-fn-name">${fn.name}</span>
                    ${badge ? `<span class="abi-badge">${badge}</span>` : ''}
                    <svg class="abi-chevron" id="${id}-chevron" viewBox="0 0 24 24" fill="none" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="6 9 12 15 18 9"/></svg>
                </div>
                <div class="abi-fn-body" id="${id}-body" style="display:none">
                    ${inputs}
                    ${btn}
                    <div class="abi-fn-result" id="${id}-result"></div>
                </div>
            </div>`;
    }).join('');
}

function toggleAbiFn(id) {
    const body    = document.getElementById(`${id}-body`);
    const chevron = document.getElementById(`${id}-chevron`);
    if (!body) return;
    const open = body.style.display !== 'none';
    body.style.display = open ? 'none' : 'block';
    if (chevron) chevron.style.transform = open ? '' : 'rotate(180deg)';
}

async function callReadFn(fnIndex) {
    if (!_interactAbi || !_interactAddress) return;
    const id = `read-fn-${fnIndex}`;
    const resultEl = document.getElementById(`${id}-result`);
    const readFns = _interactAbi.filter(f => f.type === 'function' && (f.stateMutability === 'view' || f.stateMutability === 'pure'));
    const fn = readFns[fnIndex];
    if (!fn) return;

    const args = (fn.inputs || []).map((inp, j) => parseArgValue(document.getElementById(`${id}-arg-${j}`)?.value?.trim() ?? '', inp.type));

    try {
        resultEl.className = 'abi-fn-result';
        resultEl.textContent = 'Calling...';
        if (typeof ethers === 'undefined') throw new Error('ethers.js not loaded');
        const provider = new ethers.JsonRpcProvider(getFullRpcUrl());
        const contract = new ethers.Contract(_interactAddress, _interactAbi, provider);
        const result = await contract[fn.name](...args);
        resultEl.className = 'abi-fn-result ok';
        resultEl.textContent = formatResult(result);
    } catch (err) {
        resultEl.className = 'abi-fn-result err';
        resultEl.textContent = `Error: ${err.message}`;
    }
}

async function callWriteFn(fnIndex) {
    if (!_interactAbi || !_interactAddress) return;
    const id = `write-fn-${fnIndex}`;
    const resultEl = document.getElementById(`${id}-result`);

    if (!window.connectedAddress) {
        resultEl.className = 'abi-fn-result err';
        resultEl.textContent = 'Connect your wallet to send transactions.';
        return;
    }

    const writeFns = _interactAbi.filter(f => f.type === 'function' && f.stateMutability !== 'view' && f.stateMutability !== 'pure');
    const fn = writeFns[fnIndex];
    if (!fn) return;

    const args = (fn.inputs || []).map((inp, j) => parseArgValue(document.getElementById(`${id}-arg-${j}`)?.value?.trim() ?? '', inp.type));

    try {
        resultEl.className = 'abi-fn-result';
        resultEl.textContent = 'Waiting for wallet confirmation...';
        if (typeof ethers === 'undefined') throw new Error('ethers.js not loaded');
        const provider = new ethers.BrowserProvider(window.ethereum);
        const signer = await provider.getSigner();
        const contract = new ethers.Contract(_interactAddress, _interactAbi, signer);
        const tx = await contract[fn.name](...args);
        resultEl.className = 'abi-fn-result tx';
        resultEl.textContent = `TX submitted: ${tx.hash}`;
        await tx.wait();
        resultEl.textContent = `TX confirmed: ${tx.hash}`;
    } catch (err) {
        const rejected = err.code === 4001 || err.code === 'ACTION_REJECTED';
        resultEl.className = 'abi-fn-result err';
        resultEl.textContent = rejected ? 'Transaction rejected by user.' : `Error: ${err.message}`;
    }
}

function parseArgValue(raw, type) {
    if (type === 'bool') return raw === 'true' || raw === '1';
    if (type.startsWith('uint') || type.startsWith('int')) {
        try { return BigInt(raw); } catch (_) { return raw; }
    }
    if (type.endsWith('[]') || type.includes('[')) {
        try { return JSON.parse(raw); } catch (_) { return raw; }
    }
    return raw;
}

function formatResult(val) {
    if (val === null || val === undefined) return 'null';
    if (typeof val === 'bigint') return val.toString();
    if (Array.isArray(val)) return '[' + val.map(formatResult).join(', ') + ']';
    if (typeof val === 'object') {
        try { return JSON.stringify(val, (_, v) => typeof v === 'bigint' ? v.toString() : v, 2); } catch (_) { return String(val); }
    }
    return String(val);
}

function setDevToolResult(el, message, type) {
    if (!el) return;
    el.textContent = message;
    el.className = `tool-result ${type}`;
    el.style.display = 'block';
}

// Expose dev tools functions globally
window.switchDevTab       = switchDevTab;
window.addToMetaMask      = addToMetaMask;
window.sendIDAG           = sendIDAG;
window.onDeployABIChange  = onDeployABIChange;
window.deployContract     = deployContract;
window.loadContractABI    = loadContractABI;
window.toggleAbiFn        = toggleAbiFn;
window.callReadFn         = callReadFn;
window.callWriteFn        = callWriteFn;
