import Foundation
@objc(PonteStorageHelperProtocol) protocol PonteStorageHelperProtocol {
    func publish(_ request:Data, authorization:Data, withReply reply:@escaping(Data)->Void)
    func status(withReply reply:@escaping(Data)->Void)
    func stop(withReply reply:@escaping(Data)->Void)
}
let ponteStorageService="br.com.ponte-desk.storage-helper"
func storageJSON(_ value:[String:Any])->Data{(try? JSONSerialization.data(withJSONObject:value)) ?? Data("{}".utf8)}
func storageOwnedName(_ name:String)->Bool{name.range(of:#"^PDSD_[0-9a-f]{32}$"#,options:.regularExpression) != nil}
struct StorageFailure:Error,LocalizedError {let message:String;var errorDescription:String?{message}}
func storageRun(_ executable:String,_ args:[String])throws->Data {
    let process=Process();let output=Pipe();let errors=Pipe();process.executableURL=URL(fileURLWithPath:executable);process.arguments=args;process.standardOutput=output;process.standardError=errors
    try process.run();let data=output.fileHandleForReading.readDataToEndOfFile();let err=errors.fileHandleForReading.readDataToEndOfFile();process.waitUntilExit()
    guard process.terminationStatus==0 else{throw StorageFailure(message:String(data:err,encoding:.utf8)?.trimmingCharacters(in:.whitespacesAndNewlines).prefix(300).description ?? "Falha na operação do sistema.")};return data
}
