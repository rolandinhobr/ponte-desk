#define NOMINMAX
#include <windows.h>
#include <mfapi.h>
#include <mfidl.h>
#include <mfreadwrite.h>
#include <mferror.h>
#include <wincodec.h>
#include <wrl/client.h>
#include <vector>
#include <thread>
#include <atomic>
#include <chrono>
#include <iostream>
#include <algorithm>
#include <fstream>
#include <cstdarg>
#include <cstdio>
static HANDLE diagnostic=INVALID_HANDLE_VALUE;
static void Report(const char* format,...){if(diagnostic==INVALID_HANDLE_VALUE)return;char text[1024];va_list args;va_start(args,format);int n=vsnprintf(text,sizeof(text),format,args);va_end(args);if(n>0){DWORD written;WriteFile(diagnostic,text,static_cast<DWORD>(std::min(n,1023)),&written,nullptr);}}
using Microsoft::WRL::ComPtr;
#define CHECK(hr) do{auto value=(hr);if(FAILED(value)){Report("line %d HRESULT=%08X\n",__LINE__,static_cast<UINT>(value));std::cerr<<"line "<<__LINE__<<" HRESULT 0x"<<std::hex<<value<<std::endl;return value;}}while(0)
static UINT inputWidth=1280,inputHeight=720,outputWidth=1280,outputHeight=720;static bool outputRgb=false;
using Start=HRESULT(__cdecl*)(UINT,UINT,UINT,void**);
using Push=HRESULT(__cdecl*)(void*,const BYTE*,UINT);
using Stop=HRESULT(__cdecl*)(void*);
HRESULT Jpeg(BYTE r,BYTE g,BYTE b,std::vector<BYTE>& jpeg){
 ComPtr<IWICImagingFactory> factory;CHECK(CoCreateInstance(CLSID_WICImagingFactory,nullptr,CLSCTX_INPROC_SERVER,IID_PPV_ARGS(&factory)));
 std::vector<BYTE> pixels(static_cast<size_t>(inputWidth)*inputHeight*3);for(size_t p=0;p<pixels.size();p+=3){pixels[p]=b;pixels[p+1]=g;pixels[p+2]=r;}
 ComPtr<IStream> stream;CHECK(CreateStreamOnHGlobal(nullptr,TRUE,&stream));ComPtr<IWICBitmapEncoder> encoder;CHECK(factory->CreateEncoder(GUID_ContainerFormatJpeg,nullptr,&encoder));CHECK(encoder->Initialize(stream.Get(),WICBitmapEncoderNoCache));
 ComPtr<IWICBitmapFrameEncode> frame;ComPtr<IPropertyBag2> properties;CHECK(encoder->CreateNewFrame(&frame,&properties));CHECK(frame->Initialize(properties.Get()));CHECK(frame->SetSize(inputWidth,inputHeight));WICPixelFormatGUID format=GUID_WICPixelFormat24bppBGR;CHECK(frame->SetPixelFormat(&format));CHECK(frame->WritePixels(inputHeight,inputWidth*3,static_cast<UINT>(pixels.size()),pixels.data()));CHECK(frame->Commit());CHECK(encoder->Commit());
 STATSTG stat{};CHECK(stream->Stat(&stat,STATFLAG_NONAME));jpeg.resize(static_cast<size_t>(stat.cbSize.QuadPart));LARGE_INTEGER zero{};CHECK(stream->Seek(zero,STREAM_SEEK_SET,nullptr));ULONG read;CHECK(stream->Read(jpeg.data(),static_cast<ULONG>(jpeg.size()),&read));return S_OK;
}
struct Result {std::atomic<UINT> frames{0},minY{255},maxY{0};std::atomic<HRESULT> error{S_OK};std::atomic<UINT> red{0},blue{0};std::atomic<bool> completed{false};};
HRESULT Read(Result& result){
 CHECK(CoInitializeEx(nullptr,COINIT_MULTITHREADED));CHECK(MFStartup(MF_VERSION));
 ComPtr<IMFAttributes> attrs;CHECK(MFCreateAttributes(&attrs,1));CHECK(attrs->SetGUID(MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID));
 IMFActivate** devices=nullptr;UINT count=0;CHECK(MFEnumDeviceSources(attrs.Get(),&devices,&count));ComPtr<IMFActivate> selected;
 for(UINT i=0;i<count;++i){wchar_t* name=nullptr;UINT length=0;if(SUCCEEDED(devices[i]->GetAllocatedString(MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME,&name,&length))){if(wcsstr(name,L"Ponte Desk Camera"))selected=devices[i];CoTaskMemFree(name);}devices[i]->Release();}CoTaskMemFree(devices);
 if(!selected){std::cerr<<"Ponte virtual device absent"<<std::endl;return E_FAIL;}
 ComPtr<IMFMediaSource> source;CHECK(selected->ActivateObject(IID_PPV_ARGS(&source)));ComPtr<IMFSourceReader> reader;CHECK(MFCreateSourceReaderFromMediaSource(source.Get(),nullptr,&reader));
 UINT catalog=0;
 for(DWORD index=0;index<32;++index){ComPtr<IMFMediaType> native;auto nativeResult=reader->GetNativeMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM,index,&native);if(FAILED(nativeResult))break;UINT w=0,h=0;GUID kind{};MFGetAttributeSize(native.Get(),MF_MT_FRAME_SIZE,&w,&h);native->GetGUID(MF_MT_SUBTYPE,&kind);if(w==1920&&h==1080&&kind==MFVideoFormat_NV12)catalog|=1;if(w==1920&&h==1080&&kind==MFVideoFormat_RGB32)catalog|=2;if(w==1280&&h==720&&kind==MFVideoFormat_NV12)catalog|=4;if(w==1280&&h==720&&kind==MFVideoFormat_RGB32)catalog|=8;}
 Report("native_catalog_mask=%u\n",catalog);if(catalog!=15)return MF_E_INVALIDMEDIATYPE;
 ComPtr<IMFMediaType> initial;CHECK(reader->GetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM,&initial));UINT defaultWidth=0,defaultHeight=0;MFGetAttributeSize(initial.Get(),MF_MT_FRAME_SIZE,&defaultWidth,&defaultHeight);Report("default=%ux%u\n",defaultWidth,defaultHeight);if(defaultWidth!=1920||defaultHeight!=1080)return MF_E_INVALIDMEDIATYPE;
 ComPtr<IMFMediaType> media;CHECK(MFCreateMediaType(&media));CHECK(media->SetGUID(MF_MT_MAJOR_TYPE,MFMediaType_Video));CHECK(media->SetGUID(MF_MT_SUBTYPE,outputRgb?MFVideoFormat_RGB32:MFVideoFormat_NV12));CHECK(MFSetAttributeSize(media.Get(),MF_MT_FRAME_SIZE,outputWidth,outputHeight));CHECK(MFSetAttributeRatio(media.Get(),MF_MT_FRAME_RATE,30,1));CHECK(reader->SetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM,nullptr,media.Get()));
 ComPtr<IMFMediaType> actual;CHECK(reader->GetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM,&actual));UINT width=0,height=0;CHECK(MFGetAttributeSize(actual.Get(),MF_MT_FRAME_SIZE,&width,&height));if(width!=outputWidth||height!=outputHeight)return E_UNEXPECTED;
 for(UINT i=0;i<65;++i){ComPtr<IMFSample> sample;DWORD flags=0;LONGLONG timestamp=0;CHECK(reader->ReadSample(MF_SOURCE_READER_FIRST_VIDEO_STREAM,0,nullptr,&flags,&timestamp,&sample));if(!sample)continue;ComPtr<IMFMediaBuffer> buffer;CHECK(sample->ConvertToContiguousBuffer(&buffer));BYTE* pixels=nullptr;DWORD size=0;CHECK(buffer->Lock(&pixels,nullptr,&size));if(size<outputWidth*outputHeight*(outputRgb?4:3)/ (outputRgb?1:2)){buffer->Unlock();return E_UNEXPECTED;}UINT y=outputRgb?(((66*pixels[2]+129*pixels[1]+25*pixels[0]+128)>>8)+16):pixels[0];if(y>=75&&y<=90)++result.red;if(y>=35&&y<=50)++result.blue;buffer->Unlock();result.minY.store(std::min(result.minY.load(),y));result.maxY.store(std::max(result.maxY.load(),y));++result.frames;std::this_thread::sleep_for(std::chrono::milliseconds(33));}
 source->Shutdown();reader.Reset();source.Reset();selected.Reset();actual.Reset();media.Reset();attrs.Reset();MFShutdown();CoUninitialize();return S_OK;
}
int wmain(int argc,wchar_t** argv){
 if(argc>3&&wcscmp(argv[3],L"1080")==0){inputWidth=1920;inputHeight=1080;}
 if(argc>4&&wcscmp(argv[4],L"1080")==0){outputWidth=1920;outputHeight=1080;}
 if(argc>5&&wcscmp(argv[5],L"RGB32")==0)outputRgb=true;
 if(argc>2){diagnostic=CreateFileW(argv[2],FILE_APPEND_DATA,FILE_SHARE_READ|FILE_SHARE_WRITE,nullptr,CREATE_ALWAYS,FILE_ATTRIBUTE_NORMAL,nullptr);Report("test start argc=%d\n",argc);}
 if(argc<2){std::cerr<<"Pass protected DLL path"<<std::endl;return 2;}auto dll=LoadLibraryW(argv[1]);if(!dll){Report("LoadLibrary error=%u\n",GetLastError());return 3;}Report("DLL loaded\n");
 auto start=reinterpret_cast<Start>(GetProcAddress(dll,"PonteWebcamStart"));auto push=reinterpret_cast<Push>(GetProcAddress(dll,"PonteWebcamPush"));auto stop=reinterpret_cast<Stop>(GetProcAddress(dll,"PonteWebcamStop"));if(!start||!push||!stop){Report("Missing DLL export\n");return 4;}
 void* session=nullptr;auto hr=start(inputWidth,inputHeight,30,&session);Report("Virtual camera start HRESULT=%08X\n",static_cast<UINT>(hr));if(FAILED(hr)){return 5;}
 std::vector<BYTE> red,blue;hr=Jpeg(255,0,0,red);if(SUCCEEDED(hr))hr=Jpeg(0,0,255,blue);if(FAILED(hr)){stop(session);return 6;}
 Result result;std::thread reader([&]{result.error=Read(result);result.completed=true;});UINT accepted=0;
 for(UINT i=0;i<450 && !result.completed;++i){auto& jpeg=(i/15)%2?red:blue;auto value=push(session,jpeg.data(),static_cast<UINT>(jpeg.size()));if(value==S_OK)++accepted;else if(FAILED(value))result.error=value;std::this_thread::sleep_for(std::chrono::milliseconds(33));}
 reader.join();auto stopResult=stop(session);FreeLibrary(dll);
 // Enumerate after teardown; no camera is opened by this verification.
 CoInitializeEx(nullptr,COINIT_MULTITHREADED);MFStartup(MF_VERSION);
 UINT virtualLeft=0;
 {ComPtr<IMFAttributes> attributes;MFCreateAttributes(&attributes,1);attributes->SetGUID(MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID);
  IMFActivate** found=nullptr;UINT count=0;if(SUCCEEDED(MFEnumDeviceSources(attributes.Get(),&found,&count))){for(UINT i=0;i<count;++i){wchar_t* name=nullptr;UINT n=0;if(SUCCEEDED(found[i]->GetAllocatedString(MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME,&name,&n))){if(wcsstr(name,L"Ponte Desk Camera"))++virtualLeft;CoTaskMemFree(name);}found[i]->Release();}CoTaskMemFree(found);}}
 MFShutdown();CoUninitialize();
 HANDLE token=nullptr;TOKEN_ELEVATION elevation{};DWORD bytes=0;if(OpenProcessToken(GetCurrentProcess(),TOKEN_QUERY,&token)){GetTokenInformation(token,TokenElevation,&elevation,sizeof(elevation),&bytes);CloseHandle(token);}


 Report("input_jpeg=%ux%u output=%s:%ux%u\n",inputWidth,inputHeight,outputRgb?"RGB32":"NV12",outputWidth,outputHeight);
 Report("frames=%u min=%u max=%u published=%u virtual_after_stop=%u elevated=%u red=%u blue=%u stop_hresult=%08X\n",result.frames.load(),result.minY.load(),result.maxY.load(),accepted,virtualLeft,elevation.TokenIsElevated,result.red.load(),result.blue.load(),static_cast<UINT>(stopResult));
 if(diagnostic!=INVALID_HANDLE_VALUE){CloseHandle(diagnostic);diagnostic=INVALID_HANDLE_VALUE;}
 bool passed=SUCCEEDED(result.error.load())&&result.frames>=40&&result.maxY.load()-result.minY.load()>=20&&result.red>0&&result.blue>0&&accepted>30&&SUCCEEDED(stopResult)&&virtualLeft==0;
 std::cout<<"{\"passed\":"<<(passed?"true":"false")<<",\"input_jpeg_width\":"<<inputWidth<<",\"input_jpeg_height\":"<<inputHeight<<",\"output_width\":"<<outputWidth<<",\"output_height\":"<<outputHeight<<",\"mf_frames\":"<<result.frames<<",\"min_luma\":"<<result.minY<<",\"max_luma\":"<<result.maxY<<",\"jpeg_frames_published\":"<<accepted<<",\"physical_camera_opened\":false,\"frames_written_to_disk\":false,\"virtual_devices_after_stop\":"<<virtualLeft<<",\"red_frames\":"<<result.red<<",\"blue_frames\":"<<result.blue<<",\"producer_elevated\":"<<(elevation.TokenIsElevated?"true":"false")<<"}"<<std::endl;
 return passed?0:7;
}
