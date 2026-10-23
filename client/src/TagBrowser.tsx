import { useState, useMemo, useCallback } from 'react';
import { useTagTree, useUpdateTagPath, useSensors, type TagTreeNode } from './queries';
import { inputStyle, btnStyle } from './styles';

interface TreeNodeProps {
  node: TagTreeNode;
  parentPath: string;
  selectedPath: string | null;
  onSelect: (path: string, node: TagTreeNode) => void;
  dragPath: string | null;
  onDragStart: (path: string) => void;
  onDrop: (targetPath: string) => void;
  onDragEnd: () => void;
}

function TreeNode({ node, parentPath, selectedPath, onSelect, dragPath, onDragStart, onDrop, onDragEnd }: TreeNodeProps) {
  const [expanded, setExpanded] = useState(true);
  const path = parentPath ? `${parentPath}/${node.name}` : node.name;
  const isSelected = selectedPath === path;
  const isDragging = dragPath === path;
  const hasChildren = node.children.length > 0;

  const handleDragOver = (e: React.DragEvent) => {
    e.preventDefault();
  };

  return (
    <div style={{ marginLeft: parentPath ? 16 : 0 }}>
      <div
        draggable
        onDragStart={() => onDragStart(path)}
        onDragOver={handleDragOver}
        onDrop={() => onDrop(path)}
        onDragEnd={onDragEnd}
        onClick={() => onSelect(path, node)}
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          padding: '4px 8px',
          borderRadius: 6,
          cursor: 'pointer',
          background: isSelected ? 'rgba(34,197,94,.1)' : 'transparent',
          border: isSelected ? '1px solid var(--cyan)' : '1px solid transparent',
          opacity: isDragging ? 0.4 : 1,
          transition: 'background .15s, opacity .15s',
        }}
      >
        {hasChildren && (
          <button
            onClick={e => { e.stopPropagation(); setExpanded(v => !v); }}
            style={{ background: 'none', border: 'none', color: 'var(--muted)', cursor: 'pointer', fontSize: 10, padding: 2 }}
          >
            {expanded ? '▼' : '▶'}
          </button>
        )}
        {!hasChildren && <span style={{ width: 16 }} />}
        <span style={{ fontSize: 13, fontWeight: 500, color: 'var(--fg)' }}>{node.name}</span>
        <span style={{ fontSize: 10, color: 'var(--muted)', marginLeft: 'auto' }}>
          {node.tagCount} tag{node.tagCount !== 1 ? 's' : ''}
        </span>
      </div>
      {expanded && hasChildren && (
        <div>
          {node.children.map(child => (
            <TreeNode
              key={`${path}/${child.name}`}
              node={child}
              parentPath={path}
              selectedPath={selectedPath}
              onSelect={onSelect}
              dragPath={dragPath}
              onDragStart={onDragStart}
              onDrop={onDrop}
              onDragEnd={onDragEnd}
            />
          ))}
        </div>
      )}
    </div>
  );
}

export function TagBrowser({ isAdmin }: { isAdmin: boolean }) {
  const [search, setSearch] = useState('');
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [selectedNode, setSelectedNode] = useState<TagTreeNode | null>(null);
  const [dragPath, setDragPath] = useState<string | null>(null);

  const { data, isLoading } = useTagTree(search || undefined);
  const { data: sensors } = useSensors(true);
  const updatePathMut = useUpdateTagPath();

  const tree = data?.tree ?? [];

  const relatedSensors = useMemo(() => {
    if (!selectedNode || !sensors) return [];
    const ids = new Set(selectedNode.tagIds);
    return sensors.filter(s => ids.has(s.id));
  }, [selectedNode, sensors]);

  const handleSelect = useCallback((path: string, node: TagTreeNode) => {
    setSelectedPath(path);
    setSelectedNode(node);
  }, []);

  const handleDragStart = useCallback((path: string) => {
    setDragPath(path);
  }, []);

  const collectTagIds = useCallback((node: TagTreeNode): number[] => {
    const ids = [...node.tagIds];
    for (const child of node.children) {
      ids.push(...collectTagIds(child));
    }
    return ids;
  }, []);

  const findNode = useCallback((nodes: TagTreeNode[], targetPath: string, currentPath: string): TagTreeNode | null => {
    for (const node of nodes) {
      const path = currentPath ? `${currentPath}/${node.name}` : node.name;
      if (path === targetPath) return node;
      const found = findNode(node.children, targetPath, path);
      if (found) return found;
    }
    return null;
  }, []);

  const handleDrop = useCallback((targetPath: string) => {
    if (!dragPath || dragPath === targetPath) return;
    const draggedNode = findNode(tree, dragPath, '');
    if (!draggedNode) return;
    const ids = collectTagIds(draggedNode);
    for (const id of ids) {
      updatePathMut.mutate({ id, path: targetPath || null });
    }
  }, [dragPath, tree, findNode, collectTagIds, updatePathMut]);

  const handleDragEnd = useCallback(() => {
    setDragPath(null);
  }, []);

  return (
    <div style={{ display: 'flex', height: '100%', gap: 16 }}>
      {/* Left: tree */}
      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minWidth: 0 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 12 }}>
          <input
            placeholder="Search tags..."
            value={search}
            onChange={e => setSearch(e.target.value)}
            style={{ ...inputStyle, flex: 1 }}
          />
        </div>
        <div style={{ flex: 1, overflow: 'auto', background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10, padding: 8 }}>
          {isLoading ? (
            <div style={{ color: 'var(--muted)', fontSize: 13, padding: 12 }}>Loading...</div>
          ) : tree.length === 0 ? (
            <div style={{ color: 'var(--muted)', fontSize: 13, padding: 12 }}>No tags found.</div>
          ) : (
            tree.map((node, i) => (
              <TreeNode
                key={`${node.name}-${i}`}
                node={node}
                parentPath=""
                selectedPath={selectedPath}
                onSelect={handleSelect}
                dragPath={dragPath}
                onDragStart={handleDragStart}
                onDrop={handleDrop}
                onDragEnd={handleDragEnd}
              />
            ))
          )}
        </div>
      </div>

      {/* Right: details */}
      <div style={{ width: 280, flexShrink: 0, background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10, padding: 16, display: 'flex', flexDirection: 'column', gap: 12 }}>
        <div style={{ fontSize: 14, fontWeight: 700, borderBottom: '1px solid var(--border)', paddingBottom: 8 }}>Tag Details</div>
        {!selectedNode ? (
          <div style={{ color: 'var(--muted)', fontSize: 13 }}>Select a tag to view details.</div>
        ) : (
          <>
            <div>
              <div style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', marginBottom: 4 }}>Path</div>
              <div style={{ fontSize: 13, color: 'var(--fg)', fontFamily: "var(--font-mono)", wordBreak: 'break-all' }}>{selectedPath}</div>
            </div>
            <div>
              <div style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', marginBottom: 4 }}>Tag Count</div>
              <div style={{ fontSize: 13, color: 'var(--fg)' }}>{selectedNode.tagCount}</div>
            </div>
            <div>
              <div style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', marginBottom: 4 }}>Tag IDs</div>
              <div style={{ fontSize: 12, color: 'var(--fg)', fontFamily: "var(--font-mono)", maxHeight: 120, overflow: 'auto' }}>
                {selectedNode.tagIds.length === 0 ? 'None' : selectedNode.tagIds.join(', ')}
              </div>
            </div>
            {relatedSensors.length > 0 && (
              <div>
                <div style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', marginBottom: 4 }}>Related Sensors</div>
                <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
                  {relatedSensors.map(s => (
                    <div key={s.id} style={{ fontSize: 12, color: 'var(--fg)', padding: '4px 6px', background: 'var(--bg)', borderRadius: 4 }}>
                      <span style={{ fontWeight: 600 }}>{s.name}</span>
                      <span style={{ color: 'var(--muted)', marginLeft: 4 }}>({s.key})</span>
                    </div>
                  ))}
                </div>
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );
}
