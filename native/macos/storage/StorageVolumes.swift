import Foundation
import DiskArbitration
func storageVolumes()->[[String:Any]] {
    guard let session=DASessionCreate(kCFAllocatorDefault) else{return []}
    let keys:[URLResourceKey]=[.volumeIsLocalKey,.volumeIsInternalKey,.volumeIsRemovableKey,.volumeIsEjectableKey,.volumeUUIDStringKey,.volumeNameKey,.volumeTotalCapacityKey]
    let urls=FileManager.default.mountedVolumeURLs(includingResourceValuesForKeys:keys,options:[.skipHiddenVolumes]) ?? []
    return urls.compactMap{url->[String:Any]? in
        guard url.path.hasPrefix("/Volumes/"),let values=try? url.resourceValues(forKeys:Set(keys)),values.volumeIsLocal==true,
              let disk=DADiskCreateFromVolumePath(kCFAllocatorDefault,session,url as CFURL),let desc=DADiskCopyDescription(disk) as? [String:Any] else{return nil}
        let protocolName=desc[kDADiskDescriptionDeviceProtocolKey as String] as? String ?? ""
        let removable=desc[kDADiskDescriptionMediaRemovableKey as String] as? Bool == true || values.volumeIsRemovable==true
        guard protocolName != "Disk Image",(removable || protocolName=="Secure Digital"),let id=values.volumeUUIDString,!id.isEmpty else{return nil}
        return ["id":id,"name":values.volumeName ?? url.lastPathComponent,"path":url.resolvingSymlinksInPath().path,"filesystem":desc[kDADiskDescriptionVolumeKindKey as String] as? String ?? "","bytes":values.volumeTotalCapacity ?? 0,"removable":true]
    }.prefix(16).map{$0}
}
func storageFindVolume(_ id:String)throws->[String:Any]{guard !id.isEmpty,id.count<=512,let volume=storageVolumes().first(where:{$0["id"] as? String==id}) else{throw StorageFailure(message:"O cartão selecionado não está mais conectado.")};return volume}
