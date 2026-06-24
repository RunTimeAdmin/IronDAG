import { useEffect, useRef, useState } from 'react';
import * as d3 from 'd3';
import { rpcCall } from '../lib/rpc';
import { hexToNum, RpcBlock, getStreamType } from '../lib/format';
import { DAG_REFRESH_MS } from '../lib/config';

interface DagNode extends d3.SimulationNodeDatum {
  id: string;
  blockNumber: number;
  streamType: 'A' | 'B';
  isBlue: boolean;
  isRecent: boolean;
  timestamp: number;
  x?: number;
  y?: number;
  vx?: number;
  vy?: number;
}

interface DagEdge extends d3.SimulationLinkDatum<DagNode> {
  source: string | DagNode;
  target: string | DagNode;
}

interface Props {
  onBlockClick: (hash: string) => void;
}

export function DAGVisualizer({ onBlockClick }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const svgRef = useRef<d3.Selection<SVGSVGElement, unknown, null, undefined> | null>(null);
  const simRef = useRef<d3.Simulation<DagNode, DagEdge> | null>(null);
  const zoomRef = useRef<d3.ZoomBehavior<SVGSVGElement, unknown> | null>(null);
  const nodesRef = useRef<DagNode[]>([]);
  const edgesRef = useRef<DagEdge[]>([]);
  const latestRef = useRef(0);
  const blockCacheRef = useRef(new Map<string, RpcBlock>());
  const [depth, setDepth] = useState(30);
  const [tooltip, setTooltip] = useState<{ html: string; x: number; y: number } | null>(null);

  function getNodeId(n: string | DagNode) { return typeof n === 'string' ? n : n.id; }

  async function initViz() {
    const container = containerRef.current;
    if (!container || typeof d3 === 'undefined') return;

    container.innerHTML = '<div class="dag-loading"><i class="fas fa-spinner fa-spin"></i> Loading DAG…</div>';

    try {
      const bnHex = await rpcCall<string>('eth_blockNumber');
      const latest = hexToNum(bnHex);
      latestRef.current = latest;
      const start = Math.max(0, latest - depth + 1);

      const fetched = await Promise.all(
        Array.from({ length: latest - start + 1 }, (_, i) =>
          rpcCall<RpcBlock>('eth_getBlockByNumber', [`0x${(start + i).toString(16)}`, false]).catch(() => null)
        )
      );
      const validBlocks = fetched.filter((b): b is RpcBlock => b !== null);
      if (!validBlocks.length) {
        container.innerHTML = '<div class="dag-error"><i class="fas fa-exclamation-triangle"></i> No blocks available</div>';
        return;
      }

      const knownHashes = new Set(validBlocks.map(b => b.hash));
      nodesRef.current = [];
      edgesRef.current = [];

      for (const block of validBlocks) {
        blockCacheRef.current.set(block.hash, block);
        const bn = hexToNum(block.number);
        nodesRef.current.push({
          id: block.hash,
          blockNumber: bn,
          streamType: getStreamType(block),
          isBlue: (latest - bn) >= 10,
          isRecent: (latest - bn) < 5,
          timestamp: hexToNum(block.timestamp),
          x: 100 + (bn - start) * 80,
          y: 250 + (Math.random() - 0.5) * 100,
        });
        const parents = block.parentHashes ?? block.parent_hashes ?? (block.parentHash ? [block.parentHash] : []);
        for (const ph of parents) {
          if (ph && knownHashes.has(ph)) edgesRef.current.push({ source: block.hash, target: ph });
        }
      }

      container.innerHTML = '';
      buildSvg(container, latest);
    } catch (e) {
      console.error('DAG init error:', e);
      container.innerHTML = '<div class="dag-error"><i class="fas fa-exclamation-triangle"></i> DAG unavailable</div>';
    }
  }

  function buildSvg(container: HTMLDivElement, latest: number) {
    const W = Math.max(container.clientWidth || 800, 400);
    const H = Math.max(container.clientHeight || 500, 300);

    const svg = d3.select(container).append('svg')
      .attr('width', '100%').attr('height', '100%').attr('viewBox', `0 0 ${W} ${H}`);
    svgRef.current = svg;

    const defs = svg.append('defs');
    // Stream A = Molten Amber, Stream B = Electric Cyan (IronDAG brand)
    [{ id: 'A', c1: '#fbbf24', c2: '#f59e0b' }, { id: 'B', c1: '#22d3ee', c2: '#06b6d4' }].forEach(({ id, c1, c2 }) => {
      const g = defs.append('radialGradient').attr('id', `dag-g-${id}`).attr('cx', '30%').attr('cy', '30%').attr('r', '70%');
      g.append('stop').attr('offset', '0%').attr('stop-color', c1);
      g.append('stop').attr('offset', '100%').attr('stop-color', c2);
    });

    const glow = defs.append('filter').attr('id', 'dag-glow').attr('x', '-50%').attr('y', '-50%').attr('width', '200%').attr('height', '200%');
    glow.append('feGaussianBlur').attr('stdDeviation', '3').attr('result', 'blur');
    const gm = glow.append('feMerge'); gm.append('feMergeNode').attr('in', 'blur'); gm.append('feMergeNode').attr('in', 'SourceGraphic');

    defs.append('marker').attr('id', 'dag-arrow').attr('viewBox', '0 -5 10 10').attr('refX', 20).attr('refY', 0)
      .attr('markerWidth', 6).attr('markerHeight', 6).attr('orient', 'auto')
      .append('path').attr('d', 'M0,-5L10,0L0,5').attr('class', 'dag-arrow');

    const zoom = d3.zoom<SVGSVGElement, unknown>().scaleExtent([0.1, 4]).on('zoom', e => main.attr('transform', e.transform));
    zoomRef.current = zoom;
    svg.call(zoom);

    const main = svg.append('g').attr('class', 'dag-main-group');
    main.append('g').attr('class', 'dag-edge-group');
    main.append('g').attr('class', 'dag-node-group');

    const sim = d3.forceSimulation<DagNode, DagEdge>(nodesRef.current)
      .force('link', d3.forceLink<DagNode, DagEdge>(edgesRef.current).id(d => d.id).distance(80).strength(0.5))
      .force('charge', d3.forceManyBody().strength(-150))
      .force('x', d3.forceX<DagNode>().x(d => {
        const nums = nodesRef.current.map(n => n.blockNumber);
        const mn = Math.min(...nums), mx = Math.max(...nums), rng = mx - mn || 1;
        return ((d.blockNumber - mn) / rng) * (W * 0.7) + W * 0.15;
      }).strength(0.3))
      .force('y', d3.forceY(H / 2).strength(0.1))
      .force('collision', d3.forceCollide().radius(45))
      .alphaDecay(0.05)
      .on('tick', () => {
        main.select('.dag-edge-group').selectAll('path').attr('d', (d: unknown) => {
          const edge = d as DagEdge;
          const s = typeof edge.source === 'object' ? edge.source : nodesRef.current.find(n => n.id === edge.source);
          const t = typeof edge.target === 'object' ? edge.target : nodesRef.current.find(n => n.id === edge.target);
          if (!s || !t || s.x == null || t.x == null) return '';
          const dr = Math.sqrt((t.x - s.x) ** 2 + (t.y! - s.y!) ** 2) * 0.8;
          return `M${s.x},${s.y}A${dr},${dr} 0 0,1 ${t.x},${t.y}`;
        });
        main.select('.dag-node-group').selectAll('.dag-node-group')
          .attr('transform', (d: unknown) => { const n = d as DagNode; return `translate(${n.x ?? 0},${n.y ?? 0})`; });
      });
    simRef.current = sim;

    renderNodes(latest);

    // Auto-fit after simulation settles
    setTimeout(() => {
      try {
        const bounds = (main.node() as SVGGElement).getBBox();
        if (bounds.width > 0) {
          const scale = Math.min(W / (bounds.width + 60), H / (bounds.height + 60), 1.5);
          const tx = W / 2 - (bounds.x + bounds.width / 2) * scale;
          const ty = H / 2 - (bounds.y + bounds.height / 2) * scale;
          svg.transition().duration(500).call(zoom.transform, d3.zoomIdentity.translate(tx, ty).scale(scale));
        }
      } catch { /* ignore */ }
    }, 800);
  }

  function renderNodes(latest: number) {
    if (!svgRef.current) return;
    const main = svgRef.current.select('.dag-main-group');
    const edgeGroup = main.select('.dag-edge-group');
    const nodeGroup = main.select('.dag-node-group');

    const edgePaths = edgeGroup.selectAll<SVGPathElement, DagEdge>('path').data(edgesRef.current, d => `${getNodeId(d.source)}-${getNodeId(d.target)}`);
    edgePaths.exit().remove();
    edgePaths.enter().append('path').attr('class', 'dag-edge-path')
      .attr('stroke', '#06b6d4').attr('stroke-width', 1.5).attr('stroke-opacity', 0.5)
      .attr('marker-end', 'url(#dag-arrow)').attr('fill', 'none');

    const nodeGroups = nodeGroup.selectAll<SVGGElement, DagNode>('.dag-node-group').data(nodesRef.current, d => d.id);
    nodeGroups.exit().transition().duration(300).style('opacity', 0).remove();

    const enter = nodeGroups.enter().append('g').attr('class', 'dag-node-group')
      .style('cursor', 'pointer')
      .on('click', (_, d) => onBlockClick(d.id))
      .on('mouseenter', (event: MouseEvent, d: DagNode) => {
        const ts = new Date(d.timestamp * 1000).toLocaleString();
        setTooltip({
          html: `<div class="dag-tooltip-header"><i class="fas fa-cube"></i> Block #${d.blockNumber}</div>
            <div class="dag-tooltip-row"><span class="dag-tooltip-label">Hash</span><span class="dag-tooltip-value">${d.id.slice(0, 16)}…</span></div>
            <div class="dag-tooltip-row"><span class="dag-tooltip-label">Stream</span><span class="dag-tooltip-value stream-${d.streamType.toLowerCase()}">Stream ${d.streamType}</span></div>
            <div class="dag-tooltip-row"><span class="dag-tooltip-label">Status</span><span class="dag-tooltip-status ${d.isBlue ? 'finalized' : 'pending'}">${d.isBlue ? 'Finalized' : 'Pending'}</span></div>
            <div class="dag-tooltip-row"><span class="dag-tooltip-label">Time</span><span class="dag-tooltip-value">${ts}</span></div>`,
          x: event.clientX + 15,
          y: event.clientY + 15,
        });
      })
      .on('mouseleave', () => setTooltip(null));

    enter.append('rect').attr('class', 'dag-node-rect')
      .attr('x', -35).attr('y', -18).attr('width', 70).attr('height', 36).attr('rx', 6).attr('ry', 6)
      .attr('fill', d => `url(#dag-g-${d.streamType})`)
      .attr('stroke', d => d.isBlue ? '#10b981' : '#f59e0b')
      .attr('stroke-width', 2.5).attr('filter', 'url(#dag-glow)');

    enter.append('text').attr('class', 'dag-node-text').attr('y', -3).text(d => `#${d.blockNumber}`);
    enter.append('text').attr('class', 'dag-node-hash').attr('y', 10).text(d => d.id.slice(0, 8));

    // Update existing nodes' stroke color based on finalization
    nodeGroups.merge(enter).select('.dag-node-rect')
      .attr('stroke', d => d.isBlue ? '#10b981' : '#f59e0b');

    if (simRef.current) {
      const knownSet = new Set(nodesRef.current.map(n => n.id));
      // D3 mutates source/target from string IDs → node objects; normalize back to strings
      // and drop any edges whose endpoints were pruned from the node array.
      const safeEdges: DagEdge[] = edgesRef.current
        .map(e => ({ source: getNodeId(e.source), target: getNodeId(e.target) }))
        .filter(e => knownSet.has(e.source as string) && knownSet.has(e.target as string));
      edgesRef.current = safeEdges;
      simRef.current.nodes(nodesRef.current);
      (simRef.current.force('link') as d3.ForceLink<DagNode, DagEdge>).links(safeEdges);
    }
  }

  // Initial load + depth changes
  useEffect(() => {
    svgRef.current = null;
    simRef.current = null;
    nodesRef.current = [];
    edgesRef.current = [];
    initViz();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [depth]);

  // Periodic incremental updates
  useEffect(() => {
    const id = setInterval(async () => {
      if (!svgRef.current) return;
      try {
        const bnHex = await rpcCall<string>('eth_blockNumber');
        const latest = hexToNum(bnHex);
        if (latest <= latestRef.current) return;

        const prevLatest = latestRef.current;
        latestRef.current = latest;

        const newBlocks = await Promise.all(
          Array.from({ length: latest - prevLatest }, (_, i) =>
            rpcCall<RpcBlock>('eth_getBlockByNumber', [`0x${(prevLatest + 1 + i).toString(16)}`, false]).catch(() => null)
          )
        );

        const knownIds = new Set(nodesRef.current.map(n => n.id));
        const newNodeIds = new Set<string>();

        for (const block of newBlocks) {
          if (!block) continue;
          blockCacheRef.current.set(block.hash, block);
          const bn = hexToNum(block.number);
          newNodeIds.add(block.hash);
          nodesRef.current.push({
            id: block.hash, blockNumber: bn, streamType: getStreamType(block),
            isBlue: (latest - bn) >= 10, isRecent: (latest - bn) < 5,
            timestamp: hexToNum(block.timestamp),
            x: (containerRef.current?.clientWidth ?? 800) * 0.8, y: (containerRef.current?.clientHeight ?? 500) / 2,
          });
          const parents = block.parentHashes ?? block.parent_hashes ?? (block.parentHash ? [block.parentHash] : []);
          for (const ph of parents) {
            if (ph && (knownIds.has(ph) || newNodeIds.has(ph))) edgesRef.current.push({ source: block.hash, target: ph });
          }
        }

        // Update finalization status
        nodesRef.current.forEach(n => { n.isBlue = (latest - n.blockNumber) >= 10; n.isRecent = (latest - n.blockNumber) < 5; });

        // Prune old nodes
        if (nodesRef.current.length > depth) {
          nodesRef.current.sort((a, b) => a.blockNumber - b.blockNumber);
          const removed = new Set(nodesRef.current.splice(0, nodesRef.current.length - depth).map(n => n.id));
          edgesRef.current = edgesRef.current.filter(e => !removed.has(getNodeId(e.source)) && !removed.has(getNodeId(e.target)));
        }

        renderNodes(latest);
        if (simRef.current) simRef.current.alpha(0.1).restart();
      } catch (e) { console.error('DAG update error:', e); }
    }, DAG_REFRESH_MS);
    return () => clearInterval(id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [depth]);

  function fitView() {
    if (!svgRef.current || !zoomRef.current) return;
    const main = svgRef.current.select('.dag-main-group');
    try {
      const bounds = (main.node() as SVGGElement).getBBox();
      const W = svgRef.current.node()!.clientWidth || 800;
      const H = svgRef.current.node()!.clientHeight || 500;
      const scale = Math.min(W / (bounds.width + 60), H / (bounds.height + 60), 1.5);
      const tx = W / 2 - (bounds.x + bounds.width / 2) * scale;
      const ty = H / 2 - (bounds.y + bounds.height / 2) * scale;
      svgRef.current.transition().duration(500).call(zoomRef.current.transform, d3.zoomIdentity.translate(tx, ty).scale(scale));
    } catch { /* ignore */ }
  }

  return (
    <section className="dag-explorer-section">
      <div className="container">
        <div className="section-header">
          <h2>Live BlockDAG</h2>
          <div className="dag-controls">
            <button className="dag-btn" onClick={fitView}>Fit</button>
            <button className="dag-btn" onClick={() => svgRef.current?.transition().duration(300).call(zoomRef.current!.scaleBy, 1.3)}>+</button>
            <button className="dag-btn" onClick={() => svgRef.current?.transition().duration(300).call(zoomRef.current!.scaleBy, 0.7)}>−</button>
            <select className="dag-select" value={depth} onChange={e => setDepth(parseInt(e.target.value))}>
              <option value={30}>30 blocks</option>
              <option value={50}>50 blocks</option>
              <option value={100}>100 blocks</option>
              <option value={200}>200 blocks</option>
            </select>
          </div>
        </div>
        <div ref={containerRef} id="cytoscape-dag" />
        {tooltip && (
          <div
            className="dag-tooltip"
            style={{ left: tooltip.x, top: tooltip.y, position: 'fixed', zIndex: 200 }}
            dangerouslySetInnerHTML={{ __html: tooltip.html }}
          />
        )}
        <div className="dag-legend">
          <span className="legend-item"><span className="dot stream-a" /> Stream A (Blake3)</span>
          <span className="legend-item"><span className="dot stream-b" /> Stream B (B3MemHash)</span>
          <span className="legend-item"><span className="dot blue-block" /> Blue (Finalized)</span>
          <span className="legend-item"><span className="dot red-block" /> Red (Pending)</span>
        </div>
      </div>
    </section>
  );
}
