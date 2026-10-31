// Build-only helper source; runtime performs no model loading or GPU submission.
// Vulkan entry points are loaded from System32, never the application directory.
#define VK_NO_PROTOTYPES
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <vulkan/vulkan.h>
#include <cstdio>
#include <vector>

static int unavailable(const char * reason) {
    std::printf("{\"schemaVersion\":1,\"status\":\"unavailable\",\"reason\":\"%s\"}\n", reason);
    return 0;
}
static int failure(const char * reason) {
    std::fprintf(stderr, "nero-vulkan-probe: %s\n", reason);
    return 1;
}
int main() {
    HMODULE loader = LoadLibraryExW(L"vulkan-1.dll", nullptr, LOAD_LIBRARY_SEARCH_SYSTEM32);
    if (!loader) {
        DWORD error = GetLastError();
        return error == ERROR_MOD_NOT_FOUND ? unavailable("loader-missing") : failure("loader-error");
    }
    auto get = reinterpret_cast<PFN_vkGetInstanceProcAddr>(GetProcAddress(loader, "vkGetInstanceProcAddr"));
    if (!get) return failure("loader-interface");
    auto version_fn = reinterpret_cast<PFN_vkEnumerateInstanceVersion>(get(nullptr, "vkEnumerateInstanceVersion"));
    if (!version_fn) return unavailable("vulkan-version");
    uint32_t version = 0;
    if (version_fn(&version) != VK_SUCCESS) return failure("enumerate-version");
    if (version < VK_API_VERSION_1_2) return unavailable("vulkan-version");
    auto create = reinterpret_cast<PFN_vkCreateInstance>(get(nullptr, "vkCreateInstance"));
    if (!create) return failure("instance-interface");
    VkApplicationInfo app{VK_STRUCTURE_TYPE_APPLICATION_INFO};
    app.pApplicationName = "Nero capability probe";
    app.apiVersion = VK_API_VERSION_1_2;
    VkInstanceCreateInfo info{VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO};
    info.pApplicationInfo = &app;
    VkInstance instance = VK_NULL_HANDLE;
    VkResult result = create(&info, nullptr, &instance);
    if (result == VK_ERROR_INCOMPATIBLE_DRIVER) return unavailable("incompatible-driver");
    if (result != VK_SUCCESS) return failure("create-instance");
    auto destroy = reinterpret_cast<PFN_vkDestroyInstance>(get(instance, "vkDestroyInstance"));
    auto enumerate = reinterpret_cast<PFN_vkEnumeratePhysicalDevices>(get(instance, "vkEnumeratePhysicalDevices"));
    auto properties = reinterpret_cast<PFN_vkGetPhysicalDeviceProperties>(get(instance, "vkGetPhysicalDeviceProperties"));
    auto features = reinterpret_cast<PFN_vkGetPhysicalDeviceFeatures2>(get(instance, "vkGetPhysicalDeviceFeatures2"));
    auto queues = reinterpret_cast<PFN_vkGetPhysicalDeviceQueueFamilyProperties>(get(instance, "vkGetPhysicalDeviceQueueFamilyProperties"));
    if (!destroy || !enumerate || !properties || !features || !queues) return failure("device-interface");
    uint32_t count = 0;
    if (enumerate(instance, &count, nullptr) != VK_SUCCESS || count > 256) return failure("enumerate-devices");
    std::vector<VkPhysicalDevice> devices(count);
    if (count && enumerate(instance, &count, devices.data()) != VK_SUCCESS) return failure("enumerate-devices");
    // Prefer a discrete GPU, otherwise an integrated GPU. Never a CPU/virtual device.
    for (auto type : {VK_PHYSICAL_DEVICE_TYPE_DISCRETE_GPU, VK_PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU}) {
        for (uint32_t i = 0; i < count; ++i) {
            VkPhysicalDeviceProperties props{};
            properties(devices[i], &props);
            if (props.deviceType != type || props.apiVersion < VK_API_VERSION_1_2) continue;
            VkPhysicalDeviceVulkan11Features f11{VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_1_FEATURES};
            VkPhysicalDeviceFeatures2 f2{VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FEATURES_2};
            f2.pNext = &f11;
            features(devices[i], &f2);
            if (!f11.storageBuffer16BitAccess) continue; // pinned ggml_vk_device_is_supported
            uint32_t n = 0;
            queues(devices[i], &n, nullptr);
            if (n > 256) return failure("queue-count");
            std::vector<VkQueueFamilyProperties> families(n);
            queues(devices[i], &n, families.data());
            bool compute = false;
            for (const auto & q : families) compute |= q.queueCount && (q.queueFlags & VK_QUEUE_COMPUTE_BIT);
            if (!compute) continue;
            std::printf("{\"schemaVersion\":1,\"status\":\"candidate\",\"physicalDeviceIndex\":%u,"
                        "\"vendorId\":%u,\"deviceId\":%u,\"apiVersion\":%u,\"driverVersion\":%u,"
                        "\"deviceType\":%u,\"storageBuffer16BitAccess\":true,\"computeQueue\":true}\n",
                        i, props.vendorID, props.deviceID, props.apiVersion, props.driverVersion, props.deviceType);
            destroy(instance, nullptr);
            FreeLibrary(loader);
            return 0;
        }
    }
    destroy(instance, nullptr);
    FreeLibrary(loader);
    return unavailable("no-supported-device");
}
