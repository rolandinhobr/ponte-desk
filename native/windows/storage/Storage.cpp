// Ponte Desk removable-media sharing. GPL-3.0-or-later.
#define NOMINMAX
#include <winsock2.h>
#include <windows.h>
#include <aclapi.h>
#include <lm.h>
#include <winnetwk.h>
#include <winioctl.h>
#include <sddl.h>
#include <shellapi.h>
#include <shlobj.h>
#include <netfw.h>
#include <ws2tcpip.h>
#include <roapi.h>
#include <winrt/Windows.Foundation.Collections.h>
#include <winrt/Windows.Data.Json.h>
#include <winrt/base.h>
#include <string>
#include <vector>
#include <memory>
#include <stdexcept>
#include <algorithm>
#include <sstream>
using namespace winrt::Windows::Data::Json;
using winrt::hstring;
static std::wstring get(JsonObject const& o,wchar_t const* key){return std::wstring(o.GetNamedString(key,L""));}
static void text(JsonObject& o,wchar_t const* key,std::wstring const& value){o.SetNamedValue(key,JsonValue::CreateStringValue(value));}
static void setBoolean(JsonObject& o,wchar_t const* key,bool value){o.SetNamedValue(key,JsonValue::CreateBooleanValue(value));}
static JsonObject okay(){JsonObject o;setBoolean(o,L"ok",true);return o;}
static void check(bool success,char const* what){if(!success)throw std::runtime_error(std::string(what)+" ("+std::to_string(GetLastError())+")");}
static void status(DWORD code,char const* what){if(code!=0)throw std::runtime_error(std::string(what)+" ("+std::to_string(code)+")");}
struct Handle{HANDLE value=INVALID_HANDLE_VALUE;Handle()=default;explicit Handle(HANDLE h):value(h){}~Handle(){if(value&&value!=INVALID_HANDLE_VALUE)CloseHandle(value);}Handle(Handle const&)=delete;Handle& operator=(Handle const&)=delete;bool valid()const{return value&&value!=INVALID_HANDLE_VALUE;}};
static bool session(std::wstring const& s){return s.size()==32&&std::all_of(s.begin(),s.end(),[](wchar_t c){return c>=L'0'&&c<=L'9'||c>=L'a'&&c<=L'f';});}
static bool owned(std::wstring const& s){return s.starts_with(L"PDSD_")&&session(s.substr(5));}
static void peerAddress(std::wstring const& peer){IN_ADDR address{};if(InetPtonW(AF_INET,peer.c_str(),&address)!=1)throw std::runtime_error("Invalid paired IPv4 address");}
struct Volume{std::wstring id,name,path,guid;ULONGLONG bytes=0;};
static std::vector<Volume> volumes(){
 DWORD previousMode=0;SetThreadErrorMode(SEM_FAILCRITICALERRORS|SEM_NOOPENFILEERRORBOX,&previousMode);
 struct Restore{DWORD mode;~Restore(){SetThreadErrorMode(mode,nullptr);}} restore{previousMode};
 std::vector<Volume> result;auto mask=GetLogicalDrives();
 for(wchar_t letter=L'C';letter<=L'Z'&&result.size()<16;++letter){if(!(mask&(1u<<(letter-L'A'))))continue;std::wstring root{letter,L':',L'\\'};UINT type=GetDriveTypeW(root.c_str());if(type!=DRIVE_REMOVABLE&&type!=DRIVE_FIXED)continue;
  std::wstring device=L"\\\\.\\"+root.substr(0,2);Handle disk(CreateFileW(device.c_str(),0,FILE_SHARE_READ|FILE_SHARE_WRITE|FILE_SHARE_DELETE,nullptr,OPEN_EXISTING,0,nullptr));bool card=type==DRIVE_REMOVABLE;
  if(disk.valid()){STORAGE_PROPERTY_QUERY query{};query.PropertyId=StorageDeviceProperty;query.QueryType=PropertyStandardQuery;BYTE data[4096]{};DWORD used=0;if(DeviceIoControl(disk.value,IOCTL_STORAGE_QUERY_PROPERTY,&query,sizeof(query),data,sizeof(data),&used,nullptr)&&used>=sizeof(STORAGE_DEVICE_DESCRIPTOR)){auto* d=reinterpret_cast<STORAGE_DEVICE_DESCRIPTOR*>(data);card=card||d->BusType==BusTypeSd||d->BusType==BusTypeMmc||(d->BusType==BusTypeUsb&&d->RemovableMedia);}}
  if(!card)continue;wchar_t label[256]{},fs[64]{},guid[MAX_PATH]{};DWORD serial=0;if(!GetVolumeInformationW(root.c_str(),label,256,&serial,nullptr,nullptr,fs,64)||!GetVolumeNameForVolumeMountPointW(root.c_str(),guid,MAX_PATH))continue;
  if(_wcsicmp(fs,L"CDFS")==0||_wcsicmp(fs,L"UDF")==0)continue;std::wstring guidText(guid);auto open=guidText.find(L'{'),close=guidText.find(L'}');if(open==std::wstring::npos||close==std::wstring::npos)continue;
  wchar_t serialText[10]{};swprintf_s(serialText,L"%08x",serial);ULARGE_INTEGER total{};GetDiskFreeSpaceExW(root.c_str(),nullptr,&total,nullptr);
  result.push_back({guidText.substr(open+1,close-open-1)+L"-"+serialText,label[0]?std::wstring(label):L"Cartão removível",root,guidText,total.QuadPart});
 }return result;
}
static Volume findVolume(std::wstring const& id){for(auto const& v:volumes())if(v.id==id)return v;throw std::runtime_error("The selected removable card is no longer connected");}
static std::wstring tokenSid(HANDLE process){Handle token;check(OpenProcessToken(process,TOKEN_QUERY,&token.value),"Read controller identity");DWORD length=0;GetTokenInformation(token.value,TokenUser,nullptr,0,&length);std::vector<BYTE> data(length);check(GetTokenInformation(token.value,TokenUser,data.data(),length,&length),"Read controller identity");LPWSTR sid=nullptr;check(ConvertSidToStringSidW(reinterpret_cast<TOKEN_USER*>(data.data())->User.Sid,&sid),"Read SID");std::wstring result(sid);LocalFree(sid);return result;}
static std::wstring executable(HANDLE process){wchar_t path[32768]{};DWORD length=32768;check(QueryFullProcessImageNameW(process,0,path,&length),"Read controller executable");return std::wstring(path,length);}
static std::wstring localExecutable(){return executable(GetCurrentProcess());}
static bool elevated(){Handle token;TOKEN_ELEVATION e{};DWORD size=0;check(OpenProcessToken(GetCurrentProcess(),TOKEN_QUERY,&token.value),"Read elevation");check(GetTokenInformation(token.value,TokenElevation,&e,sizeof(e),&size),"Read elevation");return e.TokenIsElevated!=0;}
static std::wstring pipeName(std::wstring const& s){return L"\\\\.\\pipe\\PonteDesk.Storage."+s;}
struct HostParent{Handle pipe,process;std::wstring id;};
static std::unique_ptr<HostParent> active;
static bool cleanupUnknown=false;
static bool io(HANDLE pipe,bool writing,std::string& data,DWORD timeout){
 Handle event(CreateEventW(nullptr,TRUE,FALSE,nullptr));OVERLAPPED ov{};ov.hEvent=event.value;DWORD used=0;BOOL done=writing?WriteFile(pipe,data.data(),static_cast<DWORD>(data.size()),&used,&ov):ReadFile(pipe,data.data(),static_cast<DWORD>(data.size()),&used,&ov);
 if(!done&&GetLastError()!=ERROR_IO_PENDING)return false;
 if(!done){if(WaitForSingleObject(event.value,timeout)!=WAIT_OBJECT_0){CancelIoEx(pipe,&ov);GetOverlappedResult(pipe,&ov,&used,TRUE);return false;}if(!GetOverlappedResult(pipe,&ov,&used,FALSE))return false;}
 if(!writing)data.resize(used);return used>0;
}
static JsonObject startHost(JsonObject const& request){
 if(active)throw std::runtime_error("A removable card is already shared");std::wstring id=get(request,L"id"),s=get(request,L"session"),peer=get(request,L"peer");if(!session(s))throw std::runtime_error("Invalid session");peerAddress(peer);findVolume(id);
 auto next=std::make_unique<HostParent>();auto sid=tokenSid(GetCurrentProcess());PSECURITY_DESCRIPTOR sd=nullptr;
 check(ConvertStringSecurityDescriptorToSecurityDescriptorW((L"D:P(A;;GA;;;"+sid+L")(A;;GA;;;SY)(A;;GA;;;BA)").c_str(),SDDL_REVISION_1,&sd,nullptr),"Protect storage IPC");SECURITY_ATTRIBUTES attrs{sizeof(attrs),sd,FALSE};
 next->pipe.value=CreateNamedPipeW(pipeName(s).c_str(),PIPE_ACCESS_DUPLEX|FILE_FLAG_OVERLAPPED|FILE_FLAG_FIRST_PIPE_INSTANCE,PIPE_TYPE_MESSAGE|PIPE_READMODE_MESSAGE|PIPE_WAIT|PIPE_REJECT_REMOTE_CLIENTS,1,16384,16384,0,&attrs);LocalFree(sd);check(next->pipe.valid(),"Create storage IPC");
 auto exe=localExecutable();std::wstring args=L"storage elevated-host --id "+id+L" --peer "+peer+L" --session "+s+L" --parent "+std::to_wstring(GetCurrentProcessId());
 SHELLEXECUTEINFOW info{};info.cbSize=sizeof(info);info.fMask=SEE_MASK_NOCLOSEPROCESS|SEE_MASK_NOASYNC;info.lpVerb=L"runas";info.lpFile=exe.c_str();info.lpParameters=args.c_str();info.nShow=SW_HIDE;check(ShellExecuteExW(&info),"Authorize card sharing");next->process.value=info.hProcess;check(next->process.valid(),"Open storage helper");cleanupUnknown=true;
 Handle event(CreateEventW(nullptr,TRUE,FALSE,nullptr));OVERLAPPED ov{};ov.hEvent=event.value;BOOL connected=ConnectNamedPipe(next->pipe.value,&ov);DWORD error=connected?ERROR_SUCCESS:GetLastError();
 if(!connected&&error==ERROR_IO_PENDING){HANDLE waits[]={event.value,next->process.value};DWORD ready=WaitForMultipleObjects(2,waits,FALSE,15000);if(ready!=WAIT_OBJECT_0){CancelIoEx(next->pipe.value,&ov);DWORD ignored;GetOverlappedResult(next->pipe.value,&ov,&ignored,TRUE);throw std::runtime_error("Storage helper did not connect");}DWORD ignored;check(GetOverlappedResult(next->pipe.value,&ov,&ignored,FALSE),"Connect storage helper");}
 else if(!connected&&error!=ERROR_PIPE_CONNECTED)throw std::runtime_error("Storage IPC connection failed");
 ULONG client=0;check(GetNamedPipeClientProcessId(next->pipe.value,&client)&&client==GetProcessId(next->process.value),"Verify storage helper identity");std::string response(16384,'\0');check(io(next->pipe.value,false,response,15000),"Read storage helper result");auto result=JsonObject::Parse(winrt::to_hstring(response));auto failure=get(result,L"error");if(!failure.empty())throw std::runtime_error(winrt::to_string(failure));next->id=id;active=std::move(next);cleanupUnknown=false;return result;
}
struct Firewall {
 winrt::com_ptr<INetFwPolicy2> policy;std::wstring name;bool added=false;
 Firewall(std::wstring const& s,std::wstring const& peer):name(L"PonteDesk.Storage."+s){
  winrt::check_hresult(CoCreateInstance(__uuidof(NetFwPolicy2),nullptr,CLSCTX_INPROC_SERVER,__uuidof(INetFwPolicy2),policy.put_void()));winrt::com_ptr<INetFwRules> rules;winrt::check_hresult(policy->get_Rules(rules.put()));winrt::com_ptr<INetFwRule> existing;BSTR key=SysAllocString(name.c_str());auto old=rules->Item(key,existing.put());SysFreeString(key);if(SUCCEEDED(old))throw std::runtime_error("Storage firewall session already exists");
  winrt::com_ptr<INetFwRule> rule;winrt::check_hresult(CoCreateInstance(__uuidof(NetFwRule),nullptr,CLSCTX_INPROC_SERVER,__uuidof(INetFwRule),rule.put_void()));
  auto prop=[](auto setter,std::wstring const& value){BSTR b=SysAllocString(value.c_str());auto hr=setter(b);SysFreeString(b);winrt::check_hresult(hr);};
  prop([&](BSTR b){return rule->put_Name(b);},name);prop([&](BSTR b){return rule->put_Description(b);},L"Ponte Desk: temporary read-only card access for the paired computer");winrt::check_hresult(rule->put_Protocol(NET_FW_IP_PROTOCOL_TCP));prop([&](BSTR b){return rule->put_LocalPorts(b);},L"445");prop([&](BSTR b){return rule->put_RemoteAddresses(b);},peer);winrt::check_hresult(rule->put_Direction(NET_FW_RULE_DIR_IN));winrt::check_hresult(rule->put_Action(NET_FW_ACTION_ALLOW));winrt::check_hresult(rule->put_Profiles(NET_FW_PROFILE2_ALL));winrt::check_hresult(rule->put_Enabled(VARIANT_TRUE));winrt::check_hresult(rules->Add(rule.get()));added=true;
 }
 ~Firewall(){if(added){winrt::com_ptr<INetFwRules> rules;if(SUCCEEDED(policy->get_Rules(rules.put()))){BSTR key=SysAllocString(name.c_str());rules->Remove(key);SysFreeString(key);}}}
};
static void secureDirectory(std::wstring const& path,std::wstring const& sid){
 // A user-writable junction must never become the target of an elevated write.
 PSECURITY_DESCRIPTOR sd=nullptr;check(ConvertStringSecurityDescriptorToSecurityDescriptorW((L"D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;GRGX;;;"+sid+L")").c_str(),SDDL_REVISION_1,&sd,nullptr),"Protect mount directory");SECURITY_ATTRIBUTES attrs{sizeof(attrs),sd,FALSE};BOOL created=CreateDirectoryW(path.c_str(),&attrs);DWORD error=GetLastError();LocalFree(sd);if(!created&&error!=ERROR_ALREADY_EXISTS)status(error,"Create protected storage directory");
 Handle dir(CreateFileW(path.c_str(),READ_CONTROL,FILE_SHARE_READ|FILE_SHARE_WRITE|FILE_SHARE_DELETE,nullptr,OPEN_EXISTING,FILE_FLAG_BACKUP_SEMANTICS|FILE_FLAG_OPEN_REPARSE_POINT,nullptr));check(dir.valid(),"Open mount directory");BY_HANDLE_FILE_INFORMATION info{};check(GetFileInformationByHandle(dir.value,&info)&&!(info.dwFileAttributes&FILE_ATTRIBUTE_REPARSE_POINT),"Reject redirected mount directory");
 PSECURITY_DESCRIPTOR actual=nullptr;PSID owner=nullptr;PACL acl=nullptr;status(GetSecurityInfo(dir.value,SE_FILE_OBJECT,OWNER_SECURITY_INFORMATION|DACL_SECURITY_INFORMATION,&owner,nullptr,&acl,nullptr,&actual),"Inspect mount owner");BYTE administrators[SECURITY_MAX_SID_SIZE],system[SECURITY_MAX_SID_SIZE];DWORD aSize=sizeof(administrators),sSize=sizeof(system);CreateWellKnownSid(WinBuiltinAdministratorsSid,nullptr,administrators,&aSize);CreateWellKnownSid(WinLocalSystemSid,nullptr,system,&sSize);bool safe=(EqualSid(owner,administrators)||EqualSid(owner,system))&&acl!=nullptr;
 if(safe){for(DWORD i=0;i<acl->AceCount;++i){void* raw=nullptr;if(!GetAce(acl,i,&raw)){safe=false;break;}auto* header=static_cast<ACE_HEADER*>(raw);if(header->AceType==ACCESS_ALLOWED_ACE_TYPE){auto* ace=static_cast<ACCESS_ALLOWED_ACE*>(raw);PSID principal=&ace->SidStart;constexpr DWORD writes=GENERIC_ALL|GENERIC_WRITE|FILE_WRITE_DATA|FILE_APPEND_DATA|FILE_WRITE_EA|FILE_WRITE_ATTRIBUTES|FILE_DELETE_CHILD|DELETE|WRITE_DAC|WRITE_OWNER;if((ace->Mask&writes)&&!EqualSid(principal,administrators)&&!EqualSid(principal,system)){safe=false;break;}}else if(header->AceType!=ACCESS_DENIED_ACE_TYPE){safe=false;break;}}}
 LocalFree(actual);if(!safe)throw std::runtime_error("Storage mount directory must be protected from non-administrator writes");
}
struct Share {
 std::wstring name,mount;bool mounted=false,shared=false;
 Share(Volume const& volume,std::wstring const& s,std::wstring const& sid):name(L"PDSD_"+s){
  PWSTR programData=nullptr;winrt::check_hresult(SHGetKnownFolderPath(FOLDERID_ProgramData,0,nullptr,&programData));std::wstring parent=std::wstring(programData)+L"\\PonteDeskStorage";CoTaskMemFree(programData);secureDirectory(parent,sid);mount=parent+L"\\"+s;secureDirectory(mount,sid);mount+=L"\\";
  try {
   check(SetVolumeMountPointW(mount.c_str(),volume.guid.c_str()),"Bind card identity");mounted=true;
   PSECURITY_DESCRIPTOR sd=nullptr;check(ConvertStringSecurityDescriptorToSecurityDescriptorW((L"D:P(A;;GRGX;;;"+sid+L")").c_str(),SDDL_REVISION_1,&sd,nullptr),"Set read-only share permissions");SHARE_INFO_502 data{};data.shi502_netname=name.data();data.shi502_type=STYPE_DISKTREE|STYPE_TEMPORARY;data.shi502_remark=const_cast<wchar_t*>(L"Ponte Desk removable card (read only)");data.shi502_max_uses=1;data.shi502_path=mount.data();data.shi502_security_descriptor=sd;DWORD parameter=0;DWORD code=NetShareAdd(nullptr,502,reinterpret_cast<BYTE*>(&data),&parameter);LocalFree(sd);status(code,"Create read-only SMB share");shared=true;
   SHARE_INFO_1005 flags{};flags.shi1005_flags=CSC_CACHE_NONE|SHI1005_FLAGS_ENCRYPT_DATA;status(NetShareSetInfo(nullptr,name.data(),1005,reinterpret_cast<BYTE*>(&flags),&parameter),"Require SMB encryption");
  }catch(...){cleanup();throw;}
 }
 DWORD cleanup(){if(shared){DWORD result=NetShareDel(nullptr,name.data(),0);if(result!=0&&result!=NERR_NetNameNotFound)return result;shared=false;}if(mounted){if(!DeleteVolumeMountPointW(mount.c_str())&&GetLastError()!=ERROR_NOT_A_REPARSE_POINT)return GetLastError();mounted=false;}if(!mount.empty()){auto dir=mount;while(dir.ends_with(L"\\"))dir.pop_back();if(!RemoveDirectoryW(dir.c_str())&&GetLastError()!=ERROR_PATH_NOT_FOUND&&GetLastError()!=ERROR_FILE_NOT_FOUND)return GetLastError();}return 0;}
 ~Share(){cleanup();}
};
static JsonObject elevatedHost(JsonObject const& request){
 if(!elevated())throw std::runtime_error("Card sharing requires explicit administrator approval");auto id=get(request,L"id"),s=get(request,L"session"),peer=get(request,L"peer");if(!session(s))throw std::runtime_error("Invalid storage session");peerAddress(peer);
 DWORD parentId=static_cast<DWORD>(request.GetNamedNumber(L"parent",0));Handle parent(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION|SYNCHRONIZE,FALSE,parentId));check(parent.valid()&&_wcsicmp(executable(parent.value).c_str(),localExecutable().c_str())==0,"Verify storage controller");auto sid=tokenSid(parent.value);
 Handle pipe(CreateFileW(pipeName(s).c_str(),GENERIC_READ|GENERIC_WRITE,0,nullptr,OPEN_EXISTING,0,nullptr));check(pipe.valid(),"Open storage controller IPC");ULONG server=0;check(GetNamedPipeServerProcessId(pipe.value,&server)&&server==parentId,"Verify storage IPC owner");
 auto send=[&](JsonObject const& value){auto bytes=winrt::to_string(value.Stringify());DWORD used=0;check(WriteFile(pipe.value,bytes.data(),static_cast<DWORD>(bytes.size()),&used,nullptr)&&used==bytes.size(),"Report storage status");};
 try {
  auto volume=findVolume(id);Share share(volume,s,sid);Firewall firewall(s,peer);JsonObject ready=okay();setBoolean(ready,L"active",true);setBoolean(ready,L"read_only",true);text(ready,L"share",share.name);send(ready);
  for(;;){bool stopping=WaitForSingleObject(parent.value,300)!=WAIT_TIMEOUT;DWORD available=0;if(!PeekNamedPipe(pipe.value,nullptr,0,nullptr,&available,nullptr))stopping=true;if(available){char command=0;DWORD used=0;if(!ReadFile(pipe.value,&command,1,&used,nullptr)||command=='S')stopping=true;}try{auto current=findVolume(id);if(current.guid!=volume.guid)stopping=true;}catch(...){stopping=true;}
   if(stopping){DWORD result=share.cleanup();if(result==0){send(okay());return okay();}JsonObject error;text(error,L"error",L"O sistema não confirmou a remoção do compartilhamento. Tente desconectar novamente.");send(error);if(WaitForSingleObject(parent.value,0)!=WAIT_TIMEOUT)return error;}
  }
 }catch(std::exception const& e){JsonObject result;text(result,L"error",winrt::to_hstring(e.what()).c_str());send(result);return result;}
}
static std::wstring mapped(std::wstring const& peer,std::wstring const& share){auto expected=L"\\\\"+peer+L"\\"+share;for(wchar_t l=L'C';l<=L'Z';++l){std::wstring letter{l,L':'};wchar_t path[2048]{};DWORD count=2048;if(WNetGetConnectionW(letter.c_str(),path,&count)==NO_ERROR&&_wcsicmp(path,expected.c_str())==0)return letter;}return L"";}
static JsonObject mountOperation(std::wstring const& op,JsonObject const& request){
 auto peer=get(request,L"peer"),share=get(request,L"share");peerAddress(peer);if(!owned(share))throw std::runtime_error("Invalid card share");auto letter=mapped(peer,share);
 if(op==L"unmount"){if(!letter.empty())status(WNetCancelConnection2W(letter.c_str(),0,FALSE),"Disconnect card (close files before disconnecting)");return okay();}
 if(op==L"mount"&&letter.empty()){
  auto used=GetLogicalDrives();for(wchar_t l=L'Z';l>=L'D';--l){wchar_t current[3]{l,L':',0};wchar_t existing[1024]{};DWORD length=1024;if(!(used&(1u<<(l-L'A')))&&WNetGetConnectionW(current,existing,&length)==ERROR_NOT_CONNECTED){letter=current;break;}}
  if(letter.empty())throw std::runtime_error("No free drive letter");std::wstring unc=L"\\\\"+peer+L"\\"+share;NETRESOURCEW resource{};resource.dwType=RESOURCETYPE_DISK;resource.lpLocalName=letter.data();resource.lpRemoteName=unc.data();DWORD code=WNetAddConnection3W(nullptr,&resource,nullptr,nullptr,CONNECT_INTERACTIVE|CONNECT_TEMPORARY);
  if(code==ERROR_SESSION_CREDENTIAL_CONFLICT)throw std::runtime_error("An existing SMB connection uses different credentials; it was left unchanged");status(code,"Mount card; use an SMB-enabled account on the source computer");
 }
 if(letter.empty())throw std::runtime_error("Card is not mounted");auto result=okay();text(result,L"location",letter+L"\\");setBoolean(result,L"read_only",true);
 if(op==L"open"){auto path=letter+L"\\";auto launched=reinterpret_cast<INT_PTR>(ShellExecuteW(nullptr,L"open",path.c_str(),nullptr,nullptr,SW_SHOWNORMAL));check(launched>32,"Open card");}return result;
}
static JsonObject dispatch(JsonObject const& request){
 auto op=get(request,L"op");
 if(op==L"list"){JsonArray list;for(auto const& v:volumes()){JsonObject item;text(item,L"id",v.id);text(item,L"name",v.name.substr(0,96));item.SetNamedValue(L"bytes",JsonValue::CreateNumberValue(static_cast<double>(v.bytes)));setBoolean(item,L"removable",true);list.Append(item);}JsonObject result;result.SetNamedValue(L"volumes",list);return result;}
 if(op==L"prepare"){auto result=okay();setBoolean(result,L"ready",true);return result;}
 if(op==L"host")return startHost(request);
 if(op==L"elevated_host")return elevatedHost(request);
 if(op==L"host_status"){if(!active||WaitForSingleObject(active->process.value,0)!=WAIT_TIMEOUT)throw std::runtime_error("Card sharing ended");findVolume(active->id);auto result=okay();setBoolean(result,L"active",true);return result;}
 if(op==L"stop_host"){if(active){std::string stop="S";io(active->pipe.value,true,stop,2000);std::string response(16384,'\0');bool done=io(active->pipe.value,false,response,10000);if(!done){cleanupUnknown=true;throw std::runtime_error("Storage helper cleanup could not be confirmed");}auto reply=JsonObject::Parse(winrt::to_hstring(response));auto failure=get(reply,L"error");if(!failure.empty())throw std::runtime_error(winrt::to_string(failure));active.reset();cleanupUnknown=false;}if(cleanupUnknown)throw std::runtime_error("Storage cleanup is unconfirmed; inspect Windows file sharing before removing the card");return okay();}
 if(op==L"mount"||op==L"unmount"||op==L"open")return mountOperation(op,request);
 throw std::runtime_error("Unknown storage operation");
}
extern "C" __declspec(dllexport) char* ponte_storage_call_json(char const* input){
 JsonObject result;HRESULT initialized=RoInitialize(RO_INIT_MULTITHREADED);bool uninit=SUCCEEDED(initialized);
 try {if(!input||strnlen_s(input,4097)>4096)throw std::runtime_error("Invalid storage request");result=dispatch(JsonObject::Parse(winrt::to_hstring(input)));}
 catch(winrt::hresult_error const& e){text(result,L"error",e.message().c_str());}catch(std::exception const& e){text(result,L"error",winrt::to_hstring(e.what()).c_str());}
 auto bytes=winrt::to_string(result.Stringify());char* output=static_cast<char*>(CoTaskMemAlloc(bytes.size()+1));if(output)memcpy(output,bytes.c_str(),bytes.size()+1);result=nullptr;if(uninit)RoUninitialize();return output;
}
extern "C" __declspec(dllexport) void ponte_storage_free(char* pointer){CoTaskMemFree(pointer);}
