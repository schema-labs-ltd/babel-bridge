import Foundation

#if os(iOS) && !APP_EXTENSION
final class BackgroundPhotoUploadSession: NSObject, URLSessionDataDelegate, URLSessionTaskDelegate, @unchecked Sendable {
    struct Result: Sendable { let data: Data; let response: URLResponse }
    static let shared = BackgroundPhotoUploadSession()

    static func cancelLegacyMonolithicUploads() {
        let configuration = URLSessionConfiguration.background(
            withIdentifier: "com.vultuk.whatsapptranslator.photo-uploads"
        )
        let legacySession = URLSession(configuration: configuration)
        legacySession.getAllTasks { tasks in
            for task in tasks {
                task.cancel()
                if let path = task.taskDescription {
                    try? FileManager.default.removeItem(atPath: path)
                }
            }
            legacySession.invalidateAndCancel()
        }
    }

    private let lock = NSLock()
    private var dataByTask: [Int: Data] = [:]
    private var continuations: [Int: CheckedContinuation<Result, Error>] = [:]
    private var tasks: [Int: URLSessionTask] = [:]
    private var completionHandler: (() -> Void)?
    private lazy var session: URLSession = {
        let configuration = URLSessionConfiguration.background(withIdentifier: "com.vultuk.whatsapptranslator.photo-uploads-v2")
        configuration.sessionSendsLaunchEvents = true
        configuration.isDiscretionary = false
        configuration.waitsForConnectivity = true
        configuration.timeoutIntervalForRequest = 120
        configuration.timeoutIntervalForResource = 20 * 60
        configuration.allowsCellularAccess = true
        configuration.allowsExpensiveNetworkAccess = true
        configuration.allowsConstrainedNetworkAccess = true
        return URLSession(configuration: configuration, delegate: self, delegateQueue: nil)
    }()

    func upload(request: URLRequest, body: Data) async throws -> Result {
        let file = FileManager.default.temporaryDirectory
            .appendingPathComponent("babel-photo-upload-\(UUID().uuidString).json")
        try body.write(to: file, options: .atomic)
        let task = session.uploadTask(with: request, fromFile: file)
        task.taskDescription = file.path
        let watchdogInterval = max(60, request.timeoutInterval + 30)
        return try await withCheckedThrowingContinuation { continuation in
            lock.lock()
            continuations[task.taskIdentifier] = continuation
            dataByTask[task.taskIdentifier] = Data()
            tasks[task.taskIdentifier] = task
            lock.unlock()
            task.resume()
            Task.detached { [weak self] in
                try? await Task.sleep(for: .seconds(watchdogInterval))
                self?.timeOut(taskIdentifier: task.taskIdentifier)
            }
        }
    }

    private func timeOut(taskIdentifier: Int) {
        lock.lock()
        let continuation = continuations.removeValue(forKey: taskIdentifier)
        let task = tasks.removeValue(forKey: taskIdentifier)
        dataByTask[taskIdentifier] = nil
        lock.unlock()
        guard let continuation else { return }
        task?.cancel()
        if let path = task?.taskDescription { try? FileManager.default.removeItem(atPath: path) }
        continuation.resume(throwing: URLError(.timedOut))
    }

    func reconnect(completionHandler: @escaping () -> Void) {
        lock.lock(); self.completionHandler = completionHandler; lock.unlock()
        _ = session
    }

    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
        lock.lock(); dataByTask[dataTask.taskIdentifier, default: Data()].append(data); lock.unlock()
    }

    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        lock.lock()
        let continuation = continuations.removeValue(forKey: task.taskIdentifier)
        tasks[task.taskIdentifier] = nil
        let data = dataByTask.removeValue(forKey: task.taskIdentifier) ?? Data()
        lock.unlock()
        if let path = task.taskDescription { try? FileManager.default.removeItem(atPath: path) }
        guard let continuation else { return }
        if let error { continuation.resume(throwing: error) }
        else if let response = task.response { continuation.resume(returning: Result(data: data, response: response)) }
        else { continuation.resume(throwing: APIError.invalidServer) }
    }

    func urlSessionDidFinishEvents(forBackgroundURLSession session: URLSession) {
        lock.lock(); let handler = completionHandler; completionHandler = nil; lock.unlock()
        DispatchQueue.main.async { handler?() }
    }
}
#endif
