import { Routes, Route } from 'react-router-dom'
import { Layout } from '@/components/Layout'
import { HomePage } from '@/pages/HomePage'
import { BlocksPage } from '@/pages/BlocksPage'
import { BlockDetailPage } from '@/pages/BlockDetailPage'
import { TxDetailPage } from '@/pages/TxDetailPage'
import { AddressPage } from '@/pages/AddressPage'
import { FaucetPage } from '@/pages/FaucetPage'
import { DevToolsPage } from '@/pages/DevToolsPage'
import { NotFoundPage } from '@/pages/NotFoundPage'

export function App() {
  return (
    <Routes>
      <Route element={<Layout />}>
        <Route index element={<HomePage />} />
        <Route path="blocks" element={<BlocksPage />} />
        <Route path="block/:id" element={<BlockDetailPage />} />
        <Route path="tx/:hash" element={<TxDetailPage />} />
        <Route path="address/:address" element={<AddressPage />} />
        <Route path="faucet" element={<FaucetPage />} />
        <Route path="dev" element={<DevToolsPage />} />
        <Route path="*" element={<NotFoundPage />} />
      </Route>
    </Routes>
  )
}
