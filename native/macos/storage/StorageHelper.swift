// On-demand privileged share administration; no input hooks or file contents.
import Foundation
import Security
import DiskArbitration
private let queue=DispatchQueue(label:"br.com.ponte-desk.storage-helper")
private let registry=URL(fileURLWithPath:"/Library/Application Support/PonteDesk/Storage")
private func secureRegistry()throws {
    let parents=[URL(fileURLWithPath:"/Library/Application Support/PonteDesk"),registry]
    for directory in parents {
        var info=stat()
        if lstat(directory.path,&info) != 0 {
            guard errno==ENOENT else{throw StorageFailure(message:"Diretório do serviço indisponível.")}
            try FileManager.default.createDirectory(at:directory,withIntermediateDirectories:false,attributes:[.posixPermissions:0o700])
        }
        guard lstat(directory.path,&info)==0,info.st_uid==0,info.st_mode & S_IFMT == S_IFDIR,info.st_mode & 0o022==0 else{throw StorageFailure(message:"O diretório do serviço não é seguro.")}
    }
}
private func shares()->[String:[String:Any]] {
    guard let data=try? storageRun("/usr/sbin/sharing",["-l","-f","json"]),let result=try? JSONSerialization.jsonObject(with:data) as? [String:[String:Any]] else{return [:]};return result
}
private func removeOwned(_ name:String,_ path:String)throws {
    guard storageOwnedName(name) else{throw StorageFailure(message:"Identidade do compartilhamento inválida.")}
    if let actual=shares()[name] {guard actual["path"] as? String==path,actual["smb_name"] as? String==name else{throw StorageFailure(message:"O compartilhamento foi alterado fora do Ponte Desk.")};_ = try storageRun("/usr/sbin/sharing",["-r",name])}
    try? FileManager.default.removeItem(at:registry.appendingPathComponent(name+".json"))
}
private func authorized(_ bytes:Data)->Bool {
    guard bytes.count==MemoryLayout<AuthorizationExternalForm>.size else{return false}
    var form=AuthorizationExternalForm();_ = withUnsafeMutableBytes(of:&form){bytes.copyBytes(to:$0)}
    var auth:AuthorizationRef?;guard AuthorizationCreateFromExternalForm(&form,&auth)==errAuthorizationSuccess,let auth else{return false};defer{AuthorizationFree(auth,[])}
    return "system.privilege.admin".withCString{right in var item=AuthorizationItem(name:right,valueLength:0,value:nil,flags:0);return withUnsafeMutablePointer(to:&item){p in var rights=AuthorizationRights(count:1,items:p);return AuthorizationCopyRights(auth,&rights,nil,[],nil)==errAuthorizationSuccess}}
}
private final class StorageSession:NSObject,PonteStorageHelperProtocol {
    private var record:[String:Any]?
    func publish(_ request:Data,authorization:Data,withReply reply:@escaping(Data)->Void){queue.async{
        do {
            guard authorized(authorization),request.count<=4096,let r=try JSONSerialization.jsonObject(with:request) as? [String:Any],let id=r["id"] as? String,let name=r["share"] as? String,storageOwnedName(name),self.record==nil else{throw StorageFailure(message:"Autorize a criação deste compartilhamento de cartão.")}
            let volume=try storageFindVolume(id);let path=volume["path"] as! String
            try secureRegistry()
            let pending=storageJSON(["share":name,"path":path,"id":id]);try pending.write(to:registry.appendingPathComponent(name+".json"),options:.atomic)
            do {_ = try storageRun("/usr/sbin/sharing",["-a",path,"-n",name,"-S",name,"-s","001","-g","000","-R","1","-E","1"])
                guard let check=shares()[name],check["path"] as? String==path,(check["smb_read_only"] as? NSNumber)?.boolValue==true,(check["smb_guest_access"] as? NSNumber)?.boolValue==false,(check["smb_sealed"] as? NSNumber)?.boolValue==true else{throw StorageFailure(message:"O sistema não confirmou o acesso somente leitura e criptografado.")}
                self.record=["share":name,"path":path,"id":id];reply(storageJSON(["ok":true,"active":true,"read_only":true]))
            }catch{try? removeOwned(name,path);throw error}
        }catch{reply(storageJSON(["error":error.localizedDescription]))}
    }}
    func status(withReply reply:@escaping(Data)->Void){queue.async{
        guard let r=self.record,let id=r["id"] as? String,(try? storageFindVolume(id)) != nil else{self.cleanup();reply(storageJSON(["error":"O cartão foi removido ou o compartilhamento terminou."]));return}
        reply(storageJSON(["active":true,"read_only":true]))
    }}
    func stop(withReply reply:@escaping(Data)->Void){queue.async{do{try self.remove();reply(storageJSON(["ok":true]))}catch{reply(storageJSON(["error":error.localizedDescription]))}}}
    private func remove()throws{if let r=record,let name=r["share"] as? String,let path=r["path"] as? String{try removeOwned(name,path);record=nil}}
    func cleanup(){try? remove()}
    func checkCard(){if let r=record,let id=r["id"] as? String,let expected=r["path"] as? String {if (try? storageFindVolume(id)["path"] as? String) != expected {cleanup()}}}
}
private final class StorageListener:NSObject,NSXPCListenerDelegate {
    private var count=0
    private var sessions=[StorageSession]()
    private var arbitration:DASession?
    override init(){super.init();arbitration=DASessionCreate(kCFAllocatorDefault);if let arbitration {let context=Unmanaged.passUnretained(self).toOpaque();DARegisterDiskDisappearedCallback(arbitration,nil,{_,context in guard let context else{return};Unmanaged<StorageListener>.fromOpaque(context).takeUnretainedValue().checkCards()},context);DARegisterDiskDescriptionChangedCallback(arbitration,nil,nil,{_,_,context in guard let context else{return};Unmanaged<StorageListener>.fromOpaque(context).takeUnretainedValue().checkCards()},context);DASessionSetDispatchQueue(arbitration,queue)}}
    func checkCards(){for session in sessions {session.checkCard()}}
    func listener(_ listener:NSXPCListener,shouldAcceptNewConnection connection:NSXPCConnection)->Bool {
        guard connection.effectiveUserIdentifier != 0 else{return false}
        let session=StorageSession();queue.sync{count+=1;sessions.append(session)}
        connection.setCodeSigningRequirement("identifier \"br.com.ponte-desk.app\"")
        connection.exportedInterface=NSXPCInterface(with:PonteStorageHelperProtocol.self);connection.exportedObject=session
        connection.invalidationHandler={queue.async{session.cleanup();self.count-=1;self.sessions.removeAll{$0 === session};if self.count==0{exit(0)}}}
        connection.resume();return true
    }
}
@main struct PonteStorageHelper {
    static func main(){
        guard geteuid()==0 else{exit(1)}
        do {try secureRegistry();for file in try FileManager.default.contentsOfDirectory(at:registry,includingPropertiesForKeys:nil) where file.pathExtension=="json" {
            if let data=try? Data(contentsOf:file),let r=try? JSONSerialization.jsonObject(with:data) as? [String:Any],let name=r["share"] as? String,let path=r["path"] as? String{try removeOwned(name,path)}
        }}catch{exit(2)}
        let listener=NSXPCListener(machServiceName:ponteStorageService);let delegate=StorageListener();listener.delegate=delegate;listener.resume();withExtendedLifetime(delegate){dispatchMain()}
    }
}
