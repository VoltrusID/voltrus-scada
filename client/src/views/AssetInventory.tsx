import { useState, useCallback } from 'react'
import { useNis2Export } from '../queries/assets'
import { AssetList } from '../components/AssetList'
import { AssetDetail } from '../components/AssetDetail'
import { SnmpScanPanel } from '../components/SnmpScanPanel'
import { AssetImportWizard } from '../components/AssetImportWizard'

export function AssetInventory() {
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [showScan, setShowScan] = useState(false)
  const [showImport, setShowImport] = useState(false)

  const nis2Export = useNis2Export()

  const handleSelectAsset = useCallback((id: string) => {
    setSelectedId(id)
  }, [])

  const handleBack = useCallback(() => {
    setSelectedId(null)
  }, [])

  const handleOpenScan = useCallback(() => {
    setShowScan(true)
  }, [])

  const handleCloseScan = useCallback(() => {
    setShowScan(false)
  }, [])

  const handleOpenImport = useCallback(() => {
    setShowImport(true)
  }, [])

  const handleCloseImport = useCallback(() => {
    setShowImport(false)
  }, [])

  const handleExportNis2 = useCallback(() => {
    nis2Export.mutate()
  }, [nis2Export])

  if (selectedId) {
    return (
      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'auto' }}>
        <AssetDetail assetId={selectedId} onBack={handleBack} />
      </div>
    )
  }

  return (
    <div style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'auto' }}>
      <AssetList
        onSelect={handleSelectAsset}
        onScan={handleOpenScan}
        onImport={handleOpenImport}
        onExport={handleExportNis2}
      />
      {showScan && <SnmpScanPanel onClose={handleCloseScan} />}
      {showImport && <AssetImportWizard onClose={handleCloseImport} />}
    </div>
  )
}
