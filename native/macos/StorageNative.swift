import AppKit
import Foundation
import NetFS
import Security
import ServiceManagement
import CryptoKit
import Darwin
private var storageConnection:NSXPCConnection?
private var storageHostError:String?
private let storageStateLock=NSLock()
private func storageProxyCall(_ connection:NSXPCConnection, timeout:Double=8,_ invoke:(PonteStorageHelperProtocol,@escaping(Data)->Void)->Void)throws->[String:Any]{
    let semaphore=DispatchSemaphore(value:0);let lock=NSLock();var bytes:Data?;var failure:String?
    let finish:(Data)->Void={data in lock.lock();bytes=data;lock.unlock();semaphore.signal()}
    guard let proxy=connection.remoteObjectProxyWithErrorHandler({error in lock.lock();failure=error.localizedDescription;lock.unlock();semaphore.signal()}) as? PonteStorageHelperProtocol else{throw StorageFailure(message:"Serviço de armazenamento indisponível.")}
    invoke(proxy,finish);guard semaphore.wait(timeout:.now()+timeout) == .success else{connection.invalidate();throw StorageFailure(message:"O auxiliar do cartão não respondeu. Use Configurar para reparar o componente antes de tentar novamente.")}
    lock.lock();defer{lock.unlock()};if let failure{throw StorageFailure(message:failure)}
    guard let bytes,let response=try JSONSerialization.jsonObject(with:bytes) as? [String:Any] else{throw StorageFailure(message:"Resposta do serviço inválida.")}
    if let error=response["error"] as? String{throw StorageFailure(message:error)};return response
}
private func storageAuthorization()throws->(AuthorizationRef,Data){
    var auth:AuthorizationRef?;guard AuthorizationCreate(nil,nil,[],&auth)==errAuthorizationSuccess,let auth else{throw StorageFailure(message:"Não foi possível iniciar a autorização.")}
    let result="system.privilege.admin".withCString{right in var item=AuthorizationItem(name:right,valueLength:0,value:nil,flags:0);return withUnsafeMutablePointer(to:&item){p in var rights=AuthorizationRights(count:1,items:p);return AuthorizationCopyRights(auth,&rights,nil,[.interactionAllowed,.extendRights,.preAuthorize],nil)}}
    guard result==errAuthorizationSuccess else{AuthorizationFree(auth,[]);throw StorageFailure(message:"A criação do compartilhamento não foi autorizada.")}
    var form=AuthorizationExternalForm();guard AuthorizationMakeExternalForm(auth,&form)==errAuthorizationSuccess else{AuthorizationFree(auth,[]);throw StorageFailure(message:"A autorização não pôde ser transmitida ao serviço.")}
    return (auth,withUnsafeBytes(of:&form){Data($0)})
}
private func storageServiceReady()throws->[String:Any] {
    let service=SMAppService.daemon(plistName:ponteStorageService+".plist")
    let root=ProcessInfo.processInfo.environment["PONTE_DESK_HOME"].map{URL(fileURLWithPath:$0)} ?? FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".config/ponte-desk")
    let receipt=root.appendingPathComponent("storage-helper-registration.json")
    let helper=Bundle.main.bundleURL.appendingPathComponent("Contents/MacOS/ponte-storage-helper")
    let digest=SHA256.hash(data:try Data(contentsOf:helper)).map{String(format:"%02x",$0)}.joined()
    let version=Bundle.main.infoDictionary?["CFBundleVersion"] as? String ?? ""
    let stamp=version+":"+digest
    let previous=(try? Data(contentsOf:receipt)).flatMap{String(data:$0,encoding:.utf8)}
    if previous != stamp && service.status != .notRegistered && service.status != .notFound {
        guard try storageShares().keys.allSatisfy({!storageOwnedName($0)}) else{throw StorageFailure(message:"Encerre o compartilhamento do cartão antes de atualizar o auxiliar.")}
        let done=DispatchSemaphore(value:0);let lock=NSLock();var failure:Error?
        service.unregister {error in lock.lock();failure=error;lock.unlock();done.signal()}
        guard done.wait(timeout:.now()+5) == .success else{throw StorageFailure(message:"O sistema ainda está atualizando o auxiliar. Tente Configurar novamente.")}
        lock.lock();let error=failure;lock.unlock();if let error{throw error}
    }
    if service.status == .notRegistered || service.status == .notFound {
        do{try service.register();try FileManager.default.createDirectory(at:root,withIntermediateDirectories:true);try Data(stamp.utf8).write(to:receipt,options:.atomic)}
        catch{if service.status == .requiresApproval || (error as NSError).code == 1{return ["ok":true,"ready":false,"approval_required":true,"message":"Ative Ponte Desk em Atividade de Apps em 2º Plano, nos Itens de Início de Sessão e Extensões."]};throw error}
    }
    guard service.status == .enabled else{return ["ok":true,"ready":false,"approval_required":service.status == .requiresApproval,"message":"Autorize o auxiliar do Ponte Desk em Itens de Início de Sessão e Extensões."]}
    let connection=NSXPCConnection(machServiceName:ponteStorageService,options:.privileged)
    connection.setCodeSigningRequirement("identifier \"br.com.ponte-desk.storage-helper\"")
    connection.remoteObjectInterface=NSXPCInterface(with:PonteStorageHelperProtocol.self);connection.resume();defer{connection.invalidate()}
    let response=try storageProxyCall(connection,timeout:5){$0.ping(withReply:$1)}
    guard response["protocol"] as? Int == 2 else{throw StorageFailure(message:"O auxiliar instalado está desatualizado. Configure o componente novamente.")}
    return ["ok":true,"ready":true,"approval_required":false,"message":"Auxiliar conectado e atualizado. O cartão pode ser compartilhado em somente leitura."]
}
private func storageMounted(_ peer:String,_ share:String)->String? {
    guard storageOwnedName(share) else{return nil}
    var mounts:UnsafeMutablePointer<statfs>?;let count=getmntinfo(&mounts,MNT_NOWAIT);guard let mounts,count>0 else{return nil}
    for index in 0..<Int(count){var info=mounts[index];let from=withUnsafePointer(to:&info.f_mntfromname){$0.withMemoryRebound(to:CChar.self,capacity:Int(MNAMELEN)){String(cString:$0)}};let path=withUnsafePointer(to:&info.f_mntonname){$0.withMemoryRebound(to:CChar.self,capacity:Int(MNAMELEN)){String(cString:$0)}}
        guard from.hasPrefix("//") else{continue};let parts=from.dropFirst(2).split(separator:"/",omittingEmptySubsequences:false);guard parts.count>=2,String(parts[1])==share else{continue}
        let host=String(parts[0]).split(separator:"@").last.map(String.init) ?? ""
        if host==peer && path.hasPrefix("/Volumes/"){return path}
    };return nil
}
private func storageNative(_ r:[String:Any])throws->[String:Any]{
    let op=r["op"] as? String ?? ""
    switch op {
    case "list":return ["volumes":storageVolumes().map{volume -> [String:Any] in var publicVolume=volume;publicVolume.removeValue(forKey:"path");publicVolume["name"]=String((volume["name"] as? String ?? "Cartão SD").prefix(96));return publicVolume},"platform":"mac"]
    case "prepare":return try storageServiceReady()
    case "reconcile_host":
        guard let share=r["share"] as? String,storageOwnedName(share) else{throw StorageFailure(message:"Sessão inválida.")}
        guard try storageShares()[share]==nil else{throw StorageFailure(message:"O compartilhamento ainda existe. Configure o auxiliar para concluir a limpeza.")}
        return ["ok":true,"cleanup_confirmed":true]
    case "host":
        guard try storageServiceReady()["ready"] as? Bool == true else{throw StorageFailure(message:"Configure e autorize o auxiliar do cartão primeiro.")}
        guard let id=r["id"] as? String,let share=r["share"] as? String,storageOwnedName(share) else{throw StorageFailure(message:"Cartão inválido.")};_ = try storageFindVolume(id)
        let (auth,data)=try storageAuthorization();defer{AuthorizationFree(auth,[])}
        let connection=NSXPCConnection(machServiceName:ponteStorageService,options:.privileged);connection.setCodeSigningRequirement("identifier \"br.com.ponte-desk.storage-helper\"");connection.remoteObjectInterface=NSXPCInterface(with:PonteStorageHelperProtocol.self)
        connection.invalidationHandler={storageStateLock.lock();storageHostError="O serviço do cartão foi interrompido.";storageStateLock.unlock()}
        connection.resume();storageConnection=connection;let result=try storageProxyCall(connection){proxy,reply in proxy.publish(storageJSON(["id":id,"share":share]),authorization:data,withReply:reply)};return result
    case "host_status":
        storageStateLock.lock();let error=storageHostError;storageStateLock.unlock();if let error{throw StorageFailure(message:error)}
        guard let connection=storageConnection else{throw StorageFailure(message:"Compartilhamento encerrado.")};return try storageProxyCall(connection){$0.status(withReply:$1)}
    case "stop_host":
        guard let connection=storageConnection else{return ["ok":true]};defer{connection.invalidate();storageConnection=nil};return try storageProxyCall(connection){$0.stop(withReply:$1)}
    case "mount","unmount","open":
        guard let peer=r["peer"] as? String,peer.range(of:#"^[0-9]{1,3}(\.[0-9]{1,3}){3}$"#,options:.regularExpression) != nil,let share=r["share"] as? String,storageOwnedName(share) else{throw StorageFailure(message:"Destino de rede inválido.")}
        if op=="unmount" {
            guard let path=storageMounted(peer,share) else{return ["ok":true]}
            // Normal unmount refuses open/busy files. Never force a desktop volume off.
            _ = try storageRun("/sbin/umount",[path]);return ["ok":true]
        }
        if op=="open" {guard let path=storageMounted(peer,share) else{throw StorageFailure(message:"O cartão não está montado.")};NSWorkspace.shared.open(URL(fileURLWithPath:path));return ["ok":true,"location":path]}
        if let path=storageMounted(peer,share){return ["ok":true,"location":path]}
        guard let url=URL(string:"smb://\(peer)/\(share)") else{throw StorageFailure(message:"Endereço SMB inválido.")}
        let mountOptions=NSMutableDictionary(dictionary:[kNetFSMountFlagsKey as String:NSNumber(value:MNT_RDONLY)]);let openOptions=NSMutableDictionary(dictionary:[kNetFSUseGuestKey as String:false])
        var points:Unmanaged<CFArray>?;let result=NetFSMountURLSync(url as CFURL,nil,nil,nil,openOptions,mountOptions,&points)
        let paths=points?.takeRetainedValue() as? [String] ?? []
        guard result==0,let path=storageMounted(peer,share),paths.contains(path) else{throw StorageFailure(message:result == -128 ? "Conexão cancelada.":"Não foi possível montar o cartão (\(result)). Confira SMB, a conta de acesso e o firewall na máquina de origem.")}
        return ["ok":true,"location":path,"read_only":true]
    default:throw StorageFailure(message:"Operação de armazenamento desconhecida.")
    }
}
@_cdecl("ponte_storage_call_json") public func ponteStorageCallJSON(_ input:UnsafePointer<CChar>?)->UnsafeMutablePointer<CChar>? {
    do{guard let input,strlen(input)<=4096,let data=String(cString:input).data(using:.utf8),let request=try JSONSerialization.jsonObject(with:data) as? [String:Any] else{throw StorageFailure(message:"Pedido inválido.")};return strdup(String(decoding:storageJSON(try storageNative(request)),as:UTF8.self))}
    catch{return strdup(String(decoding:storageJSON(["error":error.localizedDescription]),as:UTF8.self))}
}
@_cdecl("ponte_storage_free") public func ponteStorageFree(_ pointer:UnsafeMutablePointer<CChar>?){free(pointer)}
