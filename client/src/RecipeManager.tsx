import { useState } from 'react';
import { useRecipes, useCreateRecipe, useDeleteRecipe, useLoadRecipe, useUpdateRecipe } from './queries/recipes';
import { useModbusDevices } from './queries/modbus';
import { RecipeLoadStatus } from './components/RecipeLoadStatus';
import { btnStyle } from './styles';

export function RecipeManager() {
  const { data: devices } = useModbusDevices();
  const [filterDevice, setFilterDevice] = useState('');
  const [filterCategory, setFilterCategory] = useState('');
  const [showCreate, setShowCreate] = useState(false);
  const [newName, setNewName] = useState('');
  const [newDevice, setNewDevice] = useState('');
  const [newCategory, setNewCategory] = useState('');
  const [newDesc, setNewDesc] = useState('');
  const [confirmLoadId, setConfirmLoadId] = useState<number | null>(null);
  const [loadCommandIds, setLoadCommandIds] = useState<string[] | null>(null);
  const [editId, setEditId] = useState<number | null>(null);
  const [editName, setEditName] = useState('');
  const [editDesc, setEditDesc] = useState('');

  const { data: recipes, isLoading } = useRecipes(filterDevice || undefined);
  const createRecipe = useCreateRecipe();
  const deleteRecipe = useDeleteRecipe();
  const loadRecipe = useLoadRecipe();
  const updateRecipe = useUpdateRecipe();

  const categories = Array.from(new Set(recipes?.map(r => r.category).filter(Boolean) as string[]));

  const filteredRecipes = recipes?.filter(r =>
    !filterCategory || r.category === filterCategory
  ) ?? [];

  const handleCreate = () => {
    if (!newName || !newDevice) return;
    createRecipe.mutate({
      name: newName,
      device_id: newDevice,
      category: newCategory || undefined,
      description: newDesc || undefined,
    }, {
      onSuccess: () => {
        setShowCreate(false);
        setNewName('');
        setNewDevice('');
        setNewCategory('');
        setNewDesc('');
      },
    });
  };

  const handleEdit = (id: number) => {
    if (!editName) return;
    updateRecipe.mutate({
      id,
      data: {
        name: editName,
        description: editDesc || null,
      },
    }, {
      onSuccess: () => setEditId(null),
    });
  };

  const handleLoad = (id: number) => {
    loadRecipe.mutate(id, {
      onSuccess: (data) => {
        setConfirmLoadId(null);
        if (data.command_ids && data.command_ids.length > 0) {
          setLoadCommandIds(data.command_ids);
        }
      },
    });
  };

  const handleDelete = (id: number) => {
    if (confirm('Delete this recipe?')) {
      deleteRecipe.mutate(id);
    }
  };

  const startEdit = (r: { id: number; name: string; description: string | null }) => {
    setEditId(r.id);
    setEditName(r.name);
    setEditDesc(r.description || '');
  };

  return (
    <div style={{ padding: 16, display: 'flex', flexDirection: 'column', gap: 16 }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 12, flexWrap: 'wrap' }}>
        <h2 style={{ margin: 0 }}>Recipes</h2>

        <select
          value={filterDevice}
          onChange={e => setFilterDevice(e.target.value)}
          style={selectStyle}
        >
          <option value="">All devices</option>
          {devices?.map(d => (
            <option key={d.id} value={d.id}>{d.name || d.id}</option>
          ))}
        </select>

        <select
          value={filterCategory}
          onChange={e => setFilterCategory(e.target.value)}
          style={selectStyle}
        >
          <option value="">All categories</option>
          {categories.map(c => (
            <option key={c} value={c}>{c}</option>
          ))}
        </select>

        <button
          onClick={() => setShowCreate(true)}
          style={{ ...btnStyle, padding: '6px 16px', fontSize: 12 }}
        >
          + Save Current as Recipe
        </button>
      </div>

      {showCreate && (
        <div style={{
          background: 'var(--card)',
          border: '1px solid var(--border)',
          borderRadius: 8,
          padding: 16,
          display: 'flex', gap: 12, alignItems: 'flex-end', flexWrap: 'wrap',
        }}>
          <div>
            <label style={{ fontSize: 11, color: 'var(--muted)' }}>Name *</label>
            <input value={newName} onChange={e => setNewName(e.target.value)} style={inputStyle} placeholder="Recipe name" />
          </div>
          <div>
            <label style={{ fontSize: 11, color: 'var(--muted)' }}>Device *</label>
            <select value={newDevice} onChange={e => setNewDevice(e.target.value)} style={selectStyle}>
              <option value="">Select device...</option>
              {devices?.map(d => (
                <option key={d.id} value={d.id}>{d.name || d.id}</option>
              ))}
            </select>
          </div>
          <div>
            <label style={{ fontSize: 11, color: 'var(--muted)' }}>Category</label>
            <input value={newCategory} onChange={e => setNewCategory(e.target.value)} style={inputStyle} placeholder="Optional" />
          </div>
          <div>
            <label style={{ fontSize: 11, color: 'var(--muted)' }}>Description</label>
            <input value={newDesc} onChange={e => setNewDesc(e.target.value)} style={inputStyle} placeholder="Optional" />
          </div>
          <button onClick={handleCreate} style={{ ...btnStyle, padding: '6px 16px', fontSize: 12 }} disabled={createRecipe.isPending}>
            {createRecipe.isPending ? 'Saving...' : 'Save'}
          </button>
          <button onClick={() => setShowCreate(false)} style={{ ...btnStyle, padding: '6px 16px', fontSize: 12 }}>
            Cancel
          </button>
        </div>
      )}

      {isLoading && <p style={{ color: 'var(--muted)' }}>Loading recipes...</p>}

      {filteredRecipes.length === 0 && !isLoading && (
        <p style={{ color: 'var(--muted)', textAlign: 'center', marginTop: 20 }}>
          No recipes found. Save current device values as a recipe to get started.
        </p>
      )}

      {filteredRecipes.length > 0 && (
        <div style={{ overflowX: 'auto' }}>
          <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 13 }}>
            <thead>
              <tr style={{ borderBottom: '1px solid var(--border)', textAlign: 'left' }}>
                <th style={thStyle}>Name</th>
                <th style={thStyle}>Device</th>
                <th style={thStyle}>Category</th>
                <th style={thStyle}>Version</th>
                <th style={thStyle}>Description</th>
                <th style={thStyle}>Updated</th>
                <th style={{ ...thStyle, textAlign: 'right' }}>Actions</th>
              </tr>
            </thead>
            <tbody>
              {filteredRecipes.map(r => (
                <tr key={r.id} style={{ borderBottom: '1px solid var(--border)' }}>
                  {editId === r.id ? (
                    <>
                      <td style={tdStyle}>
                        <input value={editName} onChange={e => setEditName(e.target.value)} style={inputStyle} />
                      </td>
                      <td style={tdStyle}>{r.device_id}</td>
                      <td style={tdStyle}>{r.category || '--'}</td>
                      <td style={tdStyle}>v{r.version}</td>
                      <td style={tdStyle}>
                        <input value={editDesc} onChange={e => setEditDesc(e.target.value)} style={inputStyle} placeholder="Description" />
                      </td>
                      <td style={tdStyle}>{new Date(r.updated_at * 1000).toLocaleString()}</td>
                      <td style={{ ...tdStyle, textAlign: 'right' }}>
                        <button onClick={() => handleEdit(r.id)} style={actionBtn}>Save</button>
                        <button onClick={() => setEditId(null)} style={actionBtn}>Cancel</button>
                      </td>
                    </>
                  ) : (
                    <>
                      <td style={tdStyle}>{r.name}</td>
                      <td style={tdStyle}>{r.device_id}</td>
                      <td style={tdStyle}>{r.category || '--'}</td>
                      <td style={tdStyle}>
                        <span style={{
                          background: 'rgba(34,197,94,.1)',
                          color: 'var(--cyan)',
                          padding: '2px 8px',
                          borderRadius: 4,
                          fontSize: 11,
                        }}>
                          v{r.version}
                        </span>
                      </td>
                      <td style={tdStyle}>{r.description || '--'}</td>
                      <td style={tdStyle}>{new Date(r.updated_at * 1000).toLocaleString()}</td>
                      <td style={{ ...tdStyle, textAlign: 'right', whiteSpace: 'nowrap' }}>
                        <button onClick={() => setConfirmLoadId(r.id)} style={actionBtn}>Load</button>
                        <button onClick={() => startEdit(r)} style={actionBtn}>Edit</button>
                        <button onClick={() => handleDelete(r.id)} style={{ ...actionBtn, color: 'var(--crit)' }}>Delete</button>
                      </td>
                    </>
                  )}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {confirmLoadId !== null && (
        <div style={{
          position: 'fixed', inset: 0, background: 'rgba(0,0,0,0.5)',
          display: 'flex', alignItems: 'center', justifyContent: 'center', zIndex: 100,
        }}>
          <div style={{
            background: 'var(--card)', border: '1px solid var(--border)',
            borderRadius: 12, padding: 24, maxWidth: 400, width: '100%',
          }}>
            <h3 style={{ margin: '0 0 12px' }}>Load Recipe to PLC</h3>
            <p style={{ color: 'var(--muted)', fontSize: 13, marginBottom: 16 }}>
              This will write the recipe values to the target device. The actual Modbus write operations
              will be logged (dry-run mode). Are you sure?
            </p>
            {loadRecipe.isPending && (
              <p style={{ color: 'var(--cyan)', fontSize: 13, marginBottom: 12 }}>Loading recipe...</p>
            )}
            {loadRecipe.isError && (
              <p style={{ color: 'var(--crit)', fontSize: 13, marginBottom: 12 }}>
                Error: {loadRecipe.error?.message}
              </p>
            )}
            <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
              <button onClick={() => setConfirmLoadId(null)} style={btnStyle}>Cancel</button>
              <button
                onClick={() => handleLoad(confirmLoadId)}
                style={{ ...btnStyle, background: 'var(--cyan)', color: '#000' }}
                disabled={loadRecipe.isPending}
              >
                Load Recipe
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Recipe load status */}
      {loadCommandIds && (
        <RecipeLoadStatus
          commandIds={loadCommandIds}
          onComplete={() => setLoadCommandIds(null)}
        />
      )}
    </div>
  );
}

const selectStyle: React.CSSProperties = {
  background: 'var(--card)',
  border: '1px solid var(--border)',
  borderRadius: 6,
  padding: '6px 12px',
  color: 'var(--fg)',
  fontSize: 13,
};

const inputStyle: React.CSSProperties = {
  background: 'var(--card)',
  border: '1px solid var(--border)',
  borderRadius: 6,
  padding: '6px 12px',
  color: 'var(--fg)',
  fontSize: 13,
  width: 180,
  display: 'block',
};

const thStyle: React.CSSProperties = {
  padding: '8px 12px',
  fontSize: 11,
  color: 'var(--muted)',
  fontWeight: 600,
  textTransform: 'uppercase',
  letterSpacing: '0.05em',
};

const tdStyle: React.CSSProperties = {
  padding: '10px 12px',
  verticalAlign: 'middle',
};

const actionBtn: React.CSSProperties = {
  ...btnStyle,
  padding: '4px 10px',
  fontSize: 11,
  marginLeft: 4,
};
