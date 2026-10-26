#include "pch.h"
#include "Undocumented.h"
#include "Tools.h"
#include "EnumNames.h"
#include "MFTools.h"
#include "FrameGenerator.h"
#include "MediaStream.h"
#include "MediaSource.h"
#include "Activator.h"
#include <sddl.h>
#include <vector>

// 3cad447d-f283-4af4-a3b2-6f5363309f52
GUID CLSID_VCam = { 0x93e78377,0x4f8d,0x45db,{0xa4,0xb7,0x61,0x8b,0x22,0x71,0x18,0x9a} };
HMODULE _hModule;

BOOL APIENTRY DllMain(HMODULE hModule, DWORD dwReason, LPVOID lpReserved)
{
	switch (dwReason)
	{
	case DLL_PROCESS_ATTACH:
		_hModule = hModule;
		WinTraceRegister();
		WINTRACE(L"DllMain DLL_PROCESS_ATTACH '%s'", GetCommandLine());
		DisableThreadLibraryCalls(hModule);

		wil::SetResultLoggingCallback([](wil::FailureInfo const& failure) noexcept
			{
				wchar_t str[2048];
				if (SUCCEEDED(wil::GetFailureLogString(str, _countof(str), failure)))
				{
					WinTrace(2, 0, str); // 2 => error
				}
			});
		break;

	case DLL_PROCESS_DETACH:
		WINTRACE(L"DllMain DLL_PROCESS_DETACH '%s'", GetCommandLine());
		WinTraceUnregister();
		break;
	}
	return TRUE;
}

struct ClassFactory : winrt::implements<ClassFactory, IClassFactory>
{
	STDMETHODIMP CreateInstance(IUnknown* outer, GUID const& riid, void** result) noexcept final
	{
		RETURN_HR_IF_NULL(E_POINTER, result);
		*result = nullptr;
		if (outer)
			RETURN_HR(CLASS_E_NOAGGREGATION);

		auto vcam = winrt::make_self<Activator>();
		RETURN_IF_FAILED(vcam->Initialize());
		auto hr = vcam->QueryInterface(riid, result);
		if (FAILED(hr))
		{
			auto iid = GUID_ToStringW(riid);
			WINTRACE(L"ClassFactory QueryInterface failed on IID %s", iid.c_str());
		}
		return hr;
	}

	STDMETHODIMP LockServer(BOOL) noexcept final
	{
		return S_OK;
	}
};

__control_entrypoint(DllExport)
STDAPI DllCanUnloadNow()
{
	if (winrt::get_module_lock())
	{
		WINTRACE(L"DllCanUnloadNow S_FALSE");
		return S_FALSE;
	}

	winrt::clear_factory_cache();
	WINTRACE(L"DllCanUnloadNow S_OK");
	return S_OK;
}

_Check_return_
STDAPI DllGetClassObject(_In_ REFCLSID rclsid, _In_ REFIID riid, _Outptr_ LPVOID FAR* ppv)
{
	WINTRACE(L"DllGetClassObject rclsid:%s riid:%s", GUID_ToStringW(rclsid).c_str(), GUID_ToStringW(riid).c_str());
	RETURN_HR_IF_NULL(E_POINTER, ppv);
	*ppv = nullptr;

	if (rclsid == CLSID_VCam)
		return winrt::make_self<ClassFactory>()->QueryInterface(riid, ppv);

	RETURN_HR(E_NOINTERFACE);
}

using registry_key = winrt::handle_type<registry_traits>;

STDAPI DllRegisterServer()
{
	std::wstring exePath = wil::GetModuleFileNameW(_hModule).get();
	WINTRACE(L"DllRegisterServer '%s'", exePath.c_str());
	auto clsid = GUID_ToStringW(CLSID_VCam, false);
	std::wstring path = L"Software\\Classes\\CLSID\\" + clsid + L"\\InprocServer32";

	// note: a vcam *must* be registered in HKEY_LOCAL_MACHINE
	// for the frame server to be able to talk with it.
	registry_key key;
	RETURN_IF_WIN32_ERROR(RegWriteKey(HKEY_LOCAL_MACHINE, path.c_str(), key.put()));
	RETURN_IF_WIN32_ERROR(RegWriteValue(key.get(), nullptr, exePath));
	RETURN_IF_WIN32_ERROR(RegWriteValue(key.get(), L"ThreadingModel", L"Both"));
	// FrameServer runs as Local Service/System. Restrict the frame section to
    // this installing user and those service identities.
    HANDLE token = nullptr;
    RETURN_IF_WIN32_BOOL_FALSE(OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &token));
    auto closeToken = wil::scope_exit([&] { CloseHandle(token); });
    DWORD needed = 0; GetTokenInformation(token, TokenUser, nullptr, 0, &needed);
    std::vector<BYTE> data(needed);
    RETURN_IF_WIN32_BOOL_FALSE(GetTokenInformation(token, TokenUser, data.data(), needed, &needed));
    LPWSTR sid = nullptr;
    RETURN_IF_WIN32_BOOL_FALSE(ConvertSidToStringSidW(reinterpret_cast<TOKEN_USER*>(data.data())->User.Sid, &sid));
    auto freeSid = wil::scope_exit([&] { LocalFree(sid); });
    RETURN_IF_WIN32_ERROR(RegWriteValue(key.get(), L"OwnerSid", std::wstring(sid)));
    return S_OK;
}

STDAPI DllUnregisterServer()
{
	std::wstring exePath = wil::GetModuleFileNameW(_hModule).get();
	WINTRACE(L"DllUnregisterServer '%s'", exePath.c_str());
	auto clsid = GUID_ToStringW(CLSID_VCam, false);
	std::wstring path = L"Software\\Classes\\CLSID\\" + clsid;
	RETURN_IF_WIN32_ERROR(RegDeleteTree(HKEY_LOCAL_MACHINE, path.c_str()));
	return S_OK;
}