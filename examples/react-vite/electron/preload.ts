import { contextBridge, ipcRenderer } from 'electron'

contextBridge.exposeInMainWorld('api', {
  save: (name: string): Promise<string> => ipcRenderer.invoke('save', name),
})
